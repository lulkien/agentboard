#!/usr/bin/env python3
"""Sequential MCP client: drives the real binary over stdio, one request at a
time (what a real session does), and asserts on the text each tool returns."""
import json
import re
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
# The same suite runs against either build. AGENTBOARD_TEST_BIN picks the binary and
# AGENTBOARD_TEST_DB the board it uses (a surreal data directory for the surreal build,
# whose server and fresh directory the caller provides; tests/surreal.sh does). It never
# falls back to the default board: on the surreal build it refuses unless both
# AGENTBOARD_TEST_DB and AGENTBOARD_SURREAL_URL name a throwaway server, and never accepts
# the default port (the live board on a box that runs one). A sqlite binary with no
# AGENTBOARD_TEST_DB uses a throwaway .probe/client.db.
BIN = os.environ.get("AGENTBOARD_TEST_BIN") or os.path.join(ROOT, "target/release/agentboard")
EXTERNAL_BOARD = bool(os.environ.get("AGENTBOARD_TEST_DB"))
DB = os.environ.get("AGENTBOARD_TEST_DB") or os.path.join(ROOT, ".probe/client.db")
try:
    _help = subprocess.run([BIN, "--help"], capture_output=True, text=True).stdout
except OSError as e:
    sys.exit(f"tests/mcp_client.py: cannot run {BIN} ({e.strerror}); build it (cargo build --release) or set AGENTBOARD_TEST_BIN=<path>")
BACKEND = "surreal" if "Data directory of the local board server" in _help else "sqlite"
DEFAULT_PORT = "18720"
if BACKEND == "surreal":
    URL = os.environ.get("AGENTBOARD_SURREAL_URL", "")
    if not EXTERNAL_BOARD or not URL or URL.rsplit(":", 1)[-1] == DEFAULT_PORT:
        sys.exit("tests/mcp_client.py: refusing to run against the default board: the surreal build needs "
                 "AGENTBOARD_TEST_DB=<fresh data dir> and AGENTBOARD_SURREAL_URL=<throwaway server, not :18720> "
                 "(bash tests/surreal.sh sets both), or AGENTBOARD_TEST_BIN=<sqlite binary>")


class Session:
    def __init__(self, actor="claude1"):
        self.p = subprocess.Popen(
            [BIN], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
            env={**os.environ, "AGENTBOARD_DB": DB, "AGENTBOARD_ACTOR": actor},
        )
        self.id = 0
        self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": "test", "version": "0"}})
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params):
        self.id += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("server closed stdout")
            msg = json.loads(line)
            if msg.get("id") == self.id:
                if "error" in msg:
                    raise RuntimeError(msg["error"])
                return msg["result"]

    def call(self, name, **args):
        r = self.rpc("tools/call", {"name": name, "arguments": args})
        if r.get("isError"):
            raise RuntimeError(r["content"][0]["text"])
        return r["content"][0]["text"]

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=5)


def lease_mins(task_text):
    """Minutes left on the lease in a task_get render (it moves with the clock)."""
    m = re.search(r"^lease: (\d+)m left", task_text, re.M)
    return int(m.group(1)) if m else None


def stable(task_text):
    """A task_get render without what moves with the clock: the created/started/
    lease lines and the ages on event lines."""
    kept = [l for l in task_text.splitlines() if not l.startswith(("created:", "started:", "lease:", "completed:"))]
    return re.sub(r"(?m)^(\s+#\d+ \S+ \S+) \d+[smhd]:", r"\1:", "\n".join(kept))


def main():
    if not EXTERNAL_BOARD:
        shutil.rmtree(os.path.dirname(DB), ignore_errors=True)
    s = Session()
    checks = []

    def refusal(sess, **args):
        """Text of a task_claim that the server refused (an MCP error, not a reply)."""
        try:
            got = sess.call("task_claim", **args)
        except RuntimeError as e:
            return str(e)
        # The claim succeeded: that is the failure. Record it; the text is returned
        # only so the caller's needle check cannot hide it.
        checks.append(False)
        print(f"FAIL claim {args} was accepted, expected a refusal :: {got[:100]}")
        return got

    def refusal_text(call):
        """Error text of a call that must be refused; success is recorded as a failure."""
        try:
            got = call()
        except RuntimeError as e:
            return str(e)
        checks.append(False)
        print(f"FAIL call was accepted, expected a refusal :: {got[:100]}")
        return got

    def expect(label, got, *needles):
        ok = all(n in got for n in needles)
        checks.append(ok)
        print(("PASS " if ok else "FAIL ") + label + " :: " + got.replace("\n", " | ")[:120])

    expect("add", s.call("task_add", title="write fd docs", description="one page, SCM_RIGHTS", goal="verification", assignee="claude1", priority=5), "T1 ready")
    expect("list", s.call("task_list"), "T1 rdy")
    expect("claim returns description+goal", s.call("task_claim", id="T1", ttl_min=5), "T1 working", "one page, SCM_RIGHTS", "verification")
    expect("claim twice is refused", refusal(s, id="T1"), "T1 working by claude1")
    expect("note", s.call("task_update", id="T1", note="outline done"), "T1 working #")
    left = lease_mins(s.call("task_get", id="T1"))
    checks.append(left in (4, 5))
    print(("PASS " if checks[-1] else "FAIL ") + f"note does not extend lease :: {left}m left of 5")
    expect("heartbeat", s.call("task_update", id="T1", heartbeat=True), "lease")
    expect("heartbeat with note", s.call("task_update", id="T1", heartbeat=True, ttl_min=60, note="still going"), "T1 working #")
    left = lease_mins(s.call("task_get", id="T1"))
    checks.append(left is not None and 58 <= left <= 60)
    print(("PASS " if checks[-1] else "FAIL ") + f"heartbeat applied alongside note :: {left}m left of 60")
    intruder = Session(actor="intruder")
    expect("claim by other while leased is refused (internal_error, not invalid_params)", refusal(intruder, id="T1"), "-32603", "lease 60m left, until", "manager: agentboard status T1 ready")
    try:
        intruder.call("task_update", id="T1", heartbeat=True)
        expect("heartbeat by non-holder rejected", "accepted")
    except RuntimeError as e:
        expect("heartbeat by non-holder rejected", str(e), "not held by intruder")
    intruder.close()
    expect("done", s.call("task_update", id="T1", status="done", report="shipped 4f2a1c", commit="4f2a1c"), "T1 done")

    # ask must block, and wait must return on the answer without client polling.
    s.call("task_add", title="needs a decision", description="choose one", goal="decided")
    s.call("task_claim", id="T2")
    expect("ask", s.call("task_update", id="T2", status="blocking", note="nix or libc?"), "T2 blocking", "#")

    answer_at = {}

    def answer_later():
        time.sleep(1.0)
        answer_at["t"] = time.time()
        subprocess.run([BIN, "answer", "T2", "use nix"], env={**os.environ, "AGENTBOARD_DB": DB, "AGENTBOARD_ACTOR": "hermes"},
                       check=True, capture_output=True)

    import threading
    threading.Thread(target=answer_later, daemon=True).start()
    t0 = time.time()
    got = s.call("task_wait", id="T2", timeout_s=10)
    expect("wait returns on answer", got, "T2 working", "use nix")
    latency = time.time() - t0
    ok = latency < 5 and latency >= 1.0
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"wait latency {latency:.2f}s (timeout was 10s)")

    expect("brief", s.call("board_brief", since_min=60), "1 don", "T2 wrk")

    try:
        s.call("task_get", id="T77")
        print("FAIL task_get of missing task did not error")
        checks.append(False)
    except RuntimeError as e:
        ok = "no such task" in str(e)
        checks.append(ok)
        print(("PASS " if ok else "FAIL ") + f"task_get missing -> {e}")

    try:
        s.call("task_claim", id="T77")
        print("FAIL claim of missing task did not error")
        checks.append(False)
    except RuntimeError as e:
        ok = "no such task" in str(e) and "-32602" in str(e)
        checks.append(ok)
        print(("PASS " if ok else "FAIL ") + f"claim missing task stays invalid_params -> {e}")

    try:
        s.call("task_add", title="orphan", parent="T99")
        expect("missing parent rejected", "accepted")
    except RuntimeError as e:
        expect("missing parent rejected", str(e), "no such parent task T99")
    s.call("task_add", title="low", priority=50)
    s.call("task_add", title="high", priority=1)
    expect("claim next honours priority", s.call("task_claim"), "high")

    # Epic workflow: epic roundtrip, needs_fix (+ alias), children, rollup.
    epic_path = "/tmp/agentboard-no-such-project/"
    added = s.call("epic_add", title="epic: ship it", description="epic", path=epic_path)
    expect("add epic with a path that does not exist", added, "E1 open /tmp/agentboard-no-such-project", "warning: path /tmp/agentboard-no-such-project does not exist")
    epic_id = added.split()[0]
    reply = s.call("task_add", title="implement it", epic=epic_id)
    expect("add task in the epic", reply, "ready")
    child_id = reply.split()[0]
    expect("epic roundtrip on get", s.call("task_get", id=child_id), f"epic: {epic_id} epic: ship it [/tmp/agentboard-no-such-project]")
    review_id = s.call("task_add", title="review it", epic=epic_id).split()[0]
    expect("needs_fix accepted", s.call("task_update", id=child_id, status="needs_fix"), f"{child_id} needs_fix")
    s.call("task_update", id=child_id, status="working")
    expect("need fix alias accepted", s.call("task_update", id=child_id, status="need fix"), f"{child_id} needs_fix")
    got = s.call("task_get", id=child_id)
    ok = "needs_fix" in got and "lease:" not in got
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "needs_fix clears lease")
    expect("needs_fix listed as fix", s.call("task_list", status="needs-fix"), f"{child_id} fix")
    kids = s.call("task_children", id=epic_id)
    expect("children lists child", kids, "implement it", "review it")
    kid_lines = {l.split()[0]: l for l in kids.splitlines() if l.strip()}
    ok = child_id in kid_lines and " fix " in kid_lines[child_id] and review_id in kid_lines
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "children line carries fix code on the right id")
    expect("children of leaf", s.call("task_children", id=child_id), "has no children")
    expect("children of an unknown epic errors", refusal_text(lambda: s.call("task_children", id="E99")), "no such epic E99")
    for bad in ("T9999", "nonsense", ""):
        try:
            s.call("task_children", id=bad)
            expect(f"children bad id {bad!r} errors", "accepted")
        except RuntimeError as e:
            checks.append(True)
            print(f"PASS children bad id {bad!r} errors :: {e}")
    expect("needs_fix claimable only via lease msg", refusal(s, id=child_id), f"{child_id} needs_fix", "manager: agentboard status")
    ok = "(lease)" not in refusal(s, id=child_id)
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "no (lease) text for lease-less needs_fix")
    # Each move into needs_fix counts toward the reopen cap, so the aliases and the
    # cap itself are probed on a task of their own, outside the epic.
    probe = s.call("task_add", title="alias probe").split()[0]
    for alias in ("needs-fix", "fix", "Needs Fix"):
        s.call("task_update", id=probe, status="working")
        expect(f"alias {alias!r}", s.call("task_update", id=probe, status=alias), f"{probe} needs_fix")
    expect("three reopens shown as 3/3", s.call("task_get", id=probe), "reopens: 3/3")
    s.call("task_update", id=probe, status="working")
    capped = refusal_text(lambda: s.call("task_update", id=probe, status="needs_fix", note="finding four"))
    expect("fourth move into needs_fix is refused naming the cap and not_done", capped, f"{probe} reopen cap 3 hit", "not_done")
    expect("the capped row landed not_done with the body kept", s.call("task_get", id=probe), f"{probe} not_done", "reopens: 4/3 (cap hit, stopped)", "finding four")
    expect("not_done alias nd accepted", s.call("task_update", id=probe, status="nd"), f"{probe} not_done")
    s.call("task_update", id=probe, status="dropped")
    expect("needs_fix -> working re-grants lease", s.call("task_update", id=child_id, status="working"), f"{child_id} working")
    ok = "lease:" in s.call("task_get", id=child_id)
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "working after needs_fix has lease")
    s.call("task_update", id=child_id, status="needs_fix")
    lst = s.call("task_list", limit=50).splitlines()
    s.call("task_update", id=review_id, status="working")
    lst = s.call("task_list", limit=50).splitlines()
    idx = {l.split()[0]: i for i, l in enumerate(lst)}
    ok = idx[child_id] < idx[review_id]
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "list orders needs_fix above working")
    # brief lists tasks that belong to no epic; ordering is checked on two of those.
    lf = s.call("task_add", title="loose fix").split()[0]
    lw = s.call("task_add", title="loose work").split()[0]
    s.call("task_update", id=lf, status="needs_fix")
    s.call("task_claim", id=lw)
    brief = s.call("board_brief").splitlines()
    bidx = {l.split()[0]: i for i, l in enumerate(brief) if l.split()}
    ok = bidx[lf] < bidx[lw] and child_id not in bidx
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "brief orders needs_fix above working; epic tasks are not listed individually")
    s.call("task_update", id=review_id, status="ready")
    expect("epic_get shows the task counts and path", s.call("epic_get", id=epic_id), "tasks: 2 (1 needs_fix, 1 ready)", "path: /tmp/agentboard-no-such-project (does not exist)")
    expect("brief shows the epic line with its path", s.call("board_brief"), f"EPIC {epic_id} 0/2 done [1 needs_fix] /tmp/agentboard-no-such-project (does not exist) epic: ship it")

    # Dropped tasks leave the denominator; subtasks inherit the epic; closed epics drop out.
    dropped_id = s.call("task_add", title="abandoned", epic=epic_id).split()[0]
    s.call("task_update", id=dropped_id, status="dropped")
    done_id = s.call("task_add", title="finished", epic=epic_id).split()[0]
    s.call("task_update", id=done_id, status="done")
    expect("rollup excludes dropped", s.call("board_brief"), f"EPIC {epic_id} 1/3 done")
    sub_id = s.call("task_add", title="fix found by review", parent=review_id).split()[0]
    got = s.call("task_get", id=sub_id)
    expect("subtask inherits its parent's epic", got, f"epic: {epic_id}", f"parent: {review_id}")
    expect("subtask counts in the epic", s.call("epic_get", id=epic_id), "tasks: 5")
    expect("children of the parent task = its subtasks", s.call("task_children", id=review_id), sub_id, "fix found by review")
    other = s.call("epic_add", title="other epic").split()[0]
    expect("explicit epic that disagrees with the parent is refused", refusal_text(lambda: s.call("task_add", title="x", parent=review_id, epic=other)), f"not {other}")
    expect("task id for epic is refused with a hint", refusal_text(lambda: s.call("task_add", title="x", epic=review_id)), "is a task id", "E3")
    expect("epic id for parent is refused with a hint", refusal_text(lambda: s.call("task_add", title="x", parent=epic_id)), "is an epic id", "epic get")
    expect("unknown epic is refused", refusal_text(lambda: s.call("task_add", title="x", epic="E99")), "no such epic E99")
    # An epic is never leased: every task verb refuses an E id.
    for verb, args in (("task_claim", {"id": epic_id}), ("task_get", {"id": epic_id}), ("task_update", {"id": epic_id, "status": "done"}),
                       ("task_update", {"id": epic_id, "status": "blocking", "note": "q"}), ("task_wait", {"id": epic_id, "timeout_s": 1})):
        expect(f"{verb} refuses an epic id", refusal_text(lambda: s.call(verb, **args)), "is an epic id")
    expect("epic_list shows the open epic", s.call("epic_list"), f"{epic_id} open 1/4 done /tmp/agentboard-no-such-project (does not exist) epic: ship it")
    expect("epic_get refuses a task id", refusal_text(lambda: s.call("epic_get", id=child_id)), "is a task id")
    expect("epic_update closes", s.call("epic_update", id=epic_id, status="done", note="shipped"), f"{epic_id} done #")
    ok = f"{epic_id} open" not in s.call("epic_list") and "other epic" in s.call("epic_list")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "closed epic leaves epic_list; the other open epic stays")
    expect("closed epic can be listed by status", s.call("epic_list", status="done"), f"{epic_id} done")
    expect("epic_get shows its timeline", s.call("epic_get", id=epic_id), "done", "shipped", "completed:")
    expect("bad epic status refused", refusal_text(lambda: s.call("epic_update", id=epic_id, status="finished")), "bad epic status")
    ok = f"EPIC {epic_id} " not in s.call("board_brief")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "closed epic has no EPIC line")

    # Long text: plain read is marked and self-describing; full read is byte-for-byte.
    long_desc = "".join(str(i % 10) for i in range(4990)) + "ENDTAIL!!!"
    long_goal = "line one\nline two\n" + "g" * 4500 + "GOALTAIL"
    lid = s.call("task_add", title="long text", description=long_desc, goal=long_goal).split()[0]
    plain = s.call("task_get", id=lid)
    expect("get marks clipped description", plain,
           f"[truncated description: 4000 of {len(long_desc)} chars shown; full text:", f"agentboard get {lid} --full", f"task_get id={lid} full=true")
    expect("get marks clipped goal", plain, "[truncated goal: 4000 of 4530 chars shown (newline = 3 chars;", f"{len(long_goal)} raw)")
    ok = "ENDTAIL" not in plain and "GOALTAIL" not in plain
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "plain get does not leak the clipped tail")
    whole = s.call("task_get", id=lid, full=True)
    ok = f"description:\n{long_desc}\n" in whole and f"goal:\n{long_goal}\n" in whole and "[truncated" not in whole
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "task_get full=true returns description and goal byte-for-byte")
    expect("short fields carry no marker", s.call("task_get", id="T1"), "description:")
    ok = "[truncated" not in s.call("task_get", id="T1")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "short task has no truncation marker")
    claimed = s.call("task_claim", id=lid)
    ok = f"description:\n{long_desc}\n" in claimed and "[truncated" not in claimed
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "claim returns over-cap description whole")
    s.call("task_update", id=lid, report="r" * 6500 + "REPORTTAIL", note="n" * 900 + "NOTETAIL")
    plain = s.call("task_get", id=lid)
    expect("report and note marked", plain, "[truncated report: 6000 of 6510 chars shown", "[truncated note: 800 of 908 chars shown")
    whole = s.call("task_get", id=lid, full=True)
    ok = ("r" * 6500 + "REPORTTAIL") in whole and ("n" * 900 + "NOTETAIL") in whole
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "full read returns report and note whole")
    ok = len(s.call("task_list", limit=50)) < 2000 and "ENDTAIL" not in s.call("board_brief")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "list and brief stay compact")
    env = {**os.environ, "AGENTBOARD_DB": DB, "AGENTBOARD_ACTOR": "cli"}
    cli = subprocess.run([BIN, "get", lid], env=env, capture_output=True, text=True, check=True).stdout
    expect("cli get marks clip", cli, f"[truncated description: 4000 of {len(long_desc)} chars shown", "--full")
    cli_full = subprocess.run([BIN, "get", lid, "--full"], env=env, capture_output=True, text=True, check=True).stdout
    ok = f"description:\n{long_desc}\n" in cli_full and f"goal:\n{long_goal}\n" in cli_full and "[truncated" not in cli_full
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "cli get --full returns description and goal byte-for-byte")

    # Multi-line text: raw length under the cap, folded length (newline = 3 chars) over it.
    ml_desc = "a\n" * 1700
    assert len(ml_desc) < 4000
    mid = s.call("task_add", title="multi-line", description=ml_desc).split()[0]
    plain = s.call("task_get", id=mid)
    expect("multi-line get marks clip", plain, "[truncated description: 4000 of 6800 chars shown (newline = 3 chars; 3400 raw)",
           f"agentboard get {mid} --full", f"task_get id={mid} full=true")
    ok = not plain.split("[truncated")[0].rstrip().endswith("…")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "marker replaces the bare ellipsis")
    claimed = s.call("task_claim", id=mid)
    ok = f"description:\n{ml_desc}\n" in claimed and "[truncated" not in claimed
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "multi-line claim returns newlines intact")
    s.call("task_update", id=mid, status="ready")
    whole = s.call("task_get", id=mid, full=True)
    ok = f"description:\n{ml_desc}\n" in whole and "[truncated" not in whole
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "multi-line full=true returns exact bytes")
    cli_env = {**os.environ, "AGENTBOARD_DB": DB, "AGENTBOARD_ACTOR": "cli"}
    cli_ml = subprocess.run([BIN, "get", mid], env=cli_env, capture_output=True, text=True, check=True).stdout
    expect("multi-line cli get marks clip", cli_ml, "[truncated description: 4000 of 6800 chars shown")
    cli_ml_full = subprocess.run([BIN, "get", mid, "--full"], env=cli_env, capture_output=True, text=True, check=True).stdout
    ok = f"description:\n{ml_desc}\n" in cli_ml_full and "[truncated" not in cli_ml_full
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "multi-line cli --full returns exact bytes")
    # A genuine trailing ellipsis in the value must survive the marker cut.
    ell = "x" * 3999 + "…" + "y" * 10
    eid = s.call("task_add", title="ellipsis", description=ell).split()[0]
    ok = "x" * 3999 + "…[" not in s.call("task_get", id=eid) and "x" * 3999 + "… [truncated" in s.call("task_get", id=eid)
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "genuine trailing ellipsis kept before the marker")
    # Event bodies: get marks the clip, full returns them, wait never clips.
    long_ev = "e" * 500 + "EVTAIL"
    s.call("task_update", id=eid, note=long_ev)
    ev = s.call("task_get", id=eid)
    expect("get event clip is marked", ev, "[truncated: 220 of 506 chars;", f"task_get id={eid} full=true")
    ok = long_ev in s.call("task_get", id=eid, full=True)
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "get full=true returns event bodies whole")
    wait_seq = int(s.call("task_update", id=eid, note="n").split("#")[-1])
    s.call("task_update", id=eid, note=long_ev)
    expect("wait returns event body whole", s.call("task_wait", id=eid, after_seq=wait_seq, timeout_s=5), long_ev)

    # wait selectors, filters, markers and exit codes.
    def cli_wait(*args):
        return subprocess.run([BIN, "wait", *args], env=env, capture_output=True, text=True)

    wa = s.call("task_add", title="wait A").split()[0]
    wb = s.call("task_add", title="wait B").split()[0]
    s.call("task_claim", id=wa)
    s.call("task_claim", id=wb)
    s.call("diff_add", id=wa, body="fixture diff: a move to review needs one")
    # matched-on-status: another session moves the task while the wait blocks.
    mover = subprocess.Popen(["sh", "-c", f"sleep 1; {BIN} status {wa} review 'ready'"], env=env, stdout=subprocess.DEVNULL)
    t0 = time.time()
    got = s.call("task_wait", ids=[wa], status="review", timeout_s=10)
    mover.wait()
    expect("wait matched on status", got, "matched:", f"{wa} review", "review: ready")
    ok = got.startswith("matched:") and time.time() - t0 < 5
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "matched marker is the first line, returned before the timeout")
    # already satisfied: no sleep, no hang.
    t0 = time.time()
    r = cli_wait(wa, "--status", "review", "--timeout", "30")
    ok = r.returncode == 0 and r.stdout.startswith("matched:") and time.time() - t0 < 2
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"already satisfied returns at once, exit 0 :: {r.stdout.splitlines()[:2]}")
    # real (1s) timeout through the CLI: exit 3, marker, status of every watched task.
    t0 = time.time()
    r = cli_wait(wa, wb, "--status", "done", "--timeout", "1")
    ok = r.returncode == 3 and r.stdout.startswith("timeout:") and f"{wa} review" in r.stdout and f"{wb} working" in r.stdout and time.time() - t0 >= 1
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"cli timeout exits 3 after the wait :: rc={r.returncode} {r.stdout.splitlines()[:3]}")
    expect("mcp timeout marker", s.call("task_wait", ids=[wa, wb], status="done", timeout_s=1), "timeout:", "last (")
    # --open --kind ask catches a blocking update (an ask is a `block` event).
    asker = subprocess.Popen(["sh", "-c", f"sleep 1; {BIN} ask {wb} 'which one?'"], env=env, stdout=subprocess.DEVNULL)
    r = cli_wait("--open", "--kind", "ask", "--timeout", "10")
    asker.wait()
    ok = r.returncode == 0 and r.stdout.startswith("matched:") and f"{wb} blocking" in r.stdout and "block: which one?" in r.stdout and wa not in r.stdout
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"--open --kind ask catches the ask :: {r.stdout.splitlines()[:3]}")
    # fail fast.
    r = cli_wait("T9999", "--timeout", "30")
    ok = r.returncode == 1 and "no such task T9999" in r.stderr
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"unknown task id is an error :: {r.stderr.strip()}")
    r = cli_wait()
    ok = r.returncode == 1 and "--open" in r.stderr
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"no selector is an error :: {r.stderr.strip()}")
    r = cli_wait(wa, "--status", "rview")
    ok = r.returncode == 1 and "bad status" in r.stderr
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"status typo is an error :: {r.stderr.strip()}")

    # T27 fixes: labelled context line, ordering, epic/union, combined filters, dynamic --open.
    r = cli_wait(wa, "--status", "review")
    ok = r.returncode == 0 and "\nlast: #" in r.stdout and "\n#" not in r.stdout
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"status-only match labels the latest event `last:` :: {r.stdout.splitlines()[1:3]}")
    r = cli_wait(wa, "--kind", "reclaim")
    ok = r.returncode == 1 and "bad kind" in r.stderr
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"--kind reclaim rejected :: {r.stderr.strip()[:60]}")
    # Hits come back task id ascending, then event seq ascending, whatever order the ids were given.
    r = cli_wait(wb, wa, "--after", "0", "--timeout", "5")
    lines = r.stdout.splitlines()
    groups, cur = [], None
    for l in lines:
        if l.startswith("T"):
            cur = (l.split()[0], [])
            groups.append(cur)
        elif l.startswith("#"):
            cur[1].append(int(l.split()[0][1:]))
    ok = (r.returncode == 0 and [g[0] for g in groups] == sorted([wa, wb], key=lambda t: int(t[1:]))
          and all(g[1] == sorted(g[1]) and len(g[1]) >= 3 for g in groups))
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"hits ordered by task id then seq :: {groups}")
    # --epic selects children only; epic + ids is a deduplicated union.
    ep = s.call("epic_add", title="wait epic").split()[0]
    c1 = s.call("task_add", title="child 1", epic=ep).split()[0]
    c2 = s.call("task_add", title="child 2", epic=ep).split()[0]
    r = cli_wait("--epic", ep, "--status", "ready")
    heads = [l.split()[0] for l in r.stdout.splitlines() if l.startswith("T")]
    ok = r.returncode == 0 and r.stdout.startswith("matched: 2 of 2 watched") and heads == [c1, c2]
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"--epic watches the tasks in the epic :: {heads}")
    r = cli_wait(c2, "--epic", ep, c1, "--status", "ready")
    heads = [l.split()[0] for l in r.stdout.splitlines() if l.startswith("T")]
    ok = r.returncode == 0 and r.stdout.startswith("matched: 2 of 2 watched") and heads == [c1, c2]
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"epic + ids union is deduplicated :: {heads}")
    r = cli_wait(ep, "--status", "ready")
    ok = r.returncode == 1 and "is an epic id" in r.stderr
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"wait refuses an epic id as a task :: {r.stderr.strip()[:80]}")
    r = cli_wait("--epic", c1)
    ok = r.returncode == 1 and "is a task id" in r.stderr
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"wait --epic refuses a task id :: {r.stderr.strip()[:80]}")
    # --status and --kind together: both must hold.
    wc = s.call("task_add", title="wait C").split()[0]
    wd = s.call("task_add", title="wait D").split()[0]
    for t in (wc, wd):
        s.call("task_claim", id=t)
        s.call("task_update", id=t, note="progress")
    s.call("diff_add", id=wd, body="fixture diff: a move to review needs one")
    s.call("task_update", id=wd, status="review")
    r = cli_wait(wc, wd, "--status", "review", "--kind", "note", "--after", "0", "--timeout", "5")
    heads = [l.split()[0] for l in r.stdout.splitlines() if l.startswith("T")]
    ok = r.returncode == 0 and r.stdout.startswith("matched: 1 of 2 watched") and heads == [wd] and "note: progress" in r.stdout and " review: " not in r.stdout.split("\n", 2)[2]
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"--status + --kind both hold :: {heads}")
    r = cli_wait(wc, wd, "--status", "review", "--kind", "done", "--after", "0", "--timeout", "1")
    ok = r.returncode == 3
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "status holds but kind does not => timeout")
    # --open is a predicate: a task created mid-wait is watched.
    born = subprocess.Popen(["sh", "-c", f"sleep 1; {BIN} add 'born mid-wait'"], env=env, stdout=subprocess.DEVNULL)
    t0 = time.time()
    r = cli_wait("--open", "--kind", "add", "--timeout", "10")
    born.wait()
    ok = r.returncode == 0 and "add: born mid-wait" in r.stdout and time.time() - t0 < 5
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"task created during --open wait matches :: rc={r.returncode} {r.stdout.splitlines()[:3]}")

    # task_update / epic_update: amend a spec; the lease guard refuses a held task.
    ed = s.call("task_add", title="edit me", description="v1", goal="g").split()[0]
    expect("task_edit returns the changed field list, not the row", s.call("task_update", id=ed, title="edited", description="version two", priority=3), f"{ed} edited title, description, priority #")
    expect("edit lands and logs old/new lengths", s.call("task_get", id=ed), "title: edited", "priority 3", "title \"edit me\"->\"edited\", description 2->11, priority 100->3")
    expect("edit with equal values is a no-op", s.call("task_update", id=ed, title="edited"), f"{ed} unchanged")
    expect("edit with no field is refused", refusal_text(lambda: s.call("task_update", id=ed)), "nothing to do")
    s.call("task_claim", id=ed)
    before = stable(s.call("task_get", id=ed, full=True))
    expect("edit of a held task is refused like a claim (internal_error, lease text)", refusal_text(lambda: s.call("task_update", id=ed, goal="moved under the worker")), "-32603", f"{ed} working by claude1", "lease", f"manager: agentboard status {ed} ready")
    ok = stable(s.call("task_get", id=ed, full=True)) == before
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "the refused edit left the row and timeline byte-identical")
    s.call("task_update", id=ed, status="ready")
    expect("edit lands once the manager frees the task", s.call("task_update", id=ed, goal="moved"), f"{ed} edited goal")
    wd = s.call("task_add", title="workdir", path="/tmp/agentboard-wd/", branch="feat/x").split()[0]
    expect("task_add with path and branch: task_get shows both", s.call("task_get", id=wd), "path: /tmp/agentboard-wd", "branch: feat/x")
    expect("task_edit of the branch reports it", s.call("task_update", id=wd, branch="HEAD"), f"{wd} edited branch")
    expect("the branch edit logs the old->new diff", s.call("task_get", id=wd), "branch: HEAD", "branch \"feat/x\"->\"HEAD\"")
    expect("a bad branch is refused", refusal_text(lambda: s.call("task_update", id=wd, branch="a b")), "bad branch")
    expect("task_edit refuses an epic id", refusal_text(lambda: s.call("task_update", id=epic_id, title="x")), "is an epic id")
    expect("epic_edit returns the changed field list", s.call("epic_update", id=epic_id, title="epic: shipped", path="/tmp/agentboard-no-such-project/sub/"), f"{epic_id} edited title, path #", "warning: path /tmp/agentboard-no-such-project/sub does not exist")
    expect("epic edit shows in the epic and its timeline", s.call("epic_get", id=epic_id), "title: epic: shipped", "path: /tmp/agentboard-no-such-project/sub", "title \"epic: ship it\"->\"epic: shipped\", path \"/tmp/agentboard-no-such-project\"->\"/tmp/agentboard-no-such-project/sub\"")
    expect("epic_edit with no field is refused", refusal_text(lambda: s.call("epic_update", id=epic_id)), "nothing to edit")
    expect("epic_edit refuses a task id", refusal_text(lambda: s.call("epic_update", id=ed, title="x")), "is a task id")
    expect("the tools are listed", " ".join(t["name"] for t in s.rpc("tools/list", {})["tools"]), "task_update", "epic_update", "diff_add", "diff_get", "diff_list")

    # One-row epic moves use the existing setter, with no bulk or chain rewrite.
    source = s.call("epic_add", title="move source").split()[0]
    target = s.call("epic_add", title="move target").split()[0]
    moving = s.call("task_add", title="moving", epic=source, description="original", priority=8).split()[0]
    s.call("task_update", id=moving, note="before move", report="preserved report", commit="abc123")
    patch = s.call("diff_add", id=moving, body="exact preserved patch").split()[0]
    before = stable(s.call("task_get", id=moving, full=True))
    expect("task_edit epic moves with the existing setter", s.call("task_update", id=moving, epic=target), f"{moving} edited epic #")
    after = stable(s.call("task_get", id=moving, full=True))
    expect("epic move records old and new epic", after, f"epic: {target} move target", f"epic {source}->{target}")
    def move_state(record):
        return '\n'.join(line for line in record.splitlines()
                         if not line.startswith(('epic:', 'diff:'))
                         and not re.match(r'\s+#\d+ \S+ edit:', line))
    ok = move_state(before) == move_state(after) and len(re.findall(r'^  #', after, re.M)) == len(re.findall(r'^  #', before, re.M)) + 1
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "epic move preserves task fields and complete pre-move history, adds one event")
    expect("epic move preserves submitted diff", s.call("diff_get", id=patch, full=True), moving, "exact preserved patch")
    expect("same epic is a no-op", s.call("task_update", id=moving, epic=target), f"{moving} unchanged")
    unchanged = stable(s.call("task_get", id=moving, full=True))
    checks.append(move_state(unchanged) == move_state(after) and re.findall(r'#\d+', unchanged) == re.findall(r'#\d+', after))
    print(("PASS " if checks[-1] else "FAIL ") + "same-epic no-op records no event")
    s.call("epic_update", id=source, status="done")
    expect("epic move refuses done target", refusal_text(lambda: s.call("task_update", id=moving, epic=source)), "target epic is done")
    s.call("epic_update", id=source, status="dropped")
    expect("epic move refuses dropped target", refusal_text(lambda: s.call("task_update", id=moving, epic=source)), "target epic is dropped")
    expect("epic move refuses missing target", refusal_text(lambda: s.call("task_update", id=moving, epic="E999999")), "no such epic E999999")
    expect("epic move validates target id space", refusal_text(lambda: s.call("task_update", id=moving, epic=moving)), "is a task id")
    s.call("epic_update", id=source, status="open")
    s.call("task_claim", id=moving)
    expect("epic move refuses held row", refusal_text(lambda: s.call("task_update", id=moving, epic=source)), "-32603", f"{moving} working by claude1")
    s.call("task_update", id=moving, status="ready")
    child = s.call("task_add", title="move child", parent=moving).split()[0]
    grandchild = s.call("task_add", title="move grandchild", parent=child).split()[0]
    family = (moving, child, grandchild)
    family_patches = []
    for member in family:
        s.call("task_update", id=member, status="done")
        family_patches.append(s.call("diff_add", id=member, body=f"family patch {member}").split()[0])
    for member, remaining in ((moving, child), (child, grandchild)):
        expect("family row can move individually", s.call("task_update", id=member, epic=source), f"{member} edited epic #")
        expect("gap is recorded on moved row", s.call("task_get", id=member, full=True), f"family spans epics: {remaining} remains in {target}")
    expect("final family row can move", s.call("task_update", id=grandchild, epic=source), f"{grandchild} edited epic #")
    final = s.call("task_get", id=grandchild, full=True)
    checks.append('family spans epics:' not in final)
    print(("PASS " if checks[-1] else "FAIL ") + "final family move has no gap")
    for member, parent, patch_id in zip(family, (None, moving, child), family_patches):
        record = s.call("task_get", id=member, full=True)
        expect("family member arrives done", record, f"epic: {source} move source", f"{member} done")
        checks.append((f"parent: {parent}" in record) if parent else 'parent:' not in record)
        print(("PASS " if checks[-1] else "FAIL ") + "family parent preserved")
        expect("family diff preserved", s.call("diff_get", id=patch_id, full=True), member, f"family patch {member}")
    expect("middle row can split from both relatives", s.call("task_update", id=child, epic=target), f"{child} edited epic #")
    expect("one gap line names parent and child", s.call("task_get", id=child, full=True), f"family spans epics: {moving} remains in {source}; {grandchild} remains in {source}")
    s.call("task_update", id=child, epic=source)
    expect("creation still refuses cross-epic parent", refusal_text(lambda: s.call("task_add", title="invalid", parent=moving, epic=target)), f"parent {moving} belongs to {source}, not {target}")

    ungrouped = s.call("task_add", title="ungrouped parent").split()[0]
    ungrouped_child = s.call("task_add", title="ungrouped child", parent=ungrouped).split()[0]
    s.call("task_update", id=ungrouped, epic=target)
    expect("epic move notes ungrouped relative", s.call("task_get", id=ungrouped, full=True), f"family spans epics: {ungrouped_child} remains in none")

    # Diffs: append-only, round from the reopen count, only the newest is reviewed.
    dt = s.call("task_add", title="diffed").split()[0]
    expect("diff_list of a task with none", s.call("diff_list", id=dt), "(no diffs)")
    first = s.call("diff_add", id=dt, body="--- a/x\n+++ b/x\n+one")
    expect("diff_add returns D<n> (T<n> round 1)", first, f"({dt} round 1)")
    d1 = first.split()[0]
    expect("diff_get shows task, round and the body", s.call("diff_get", id=d1), f"{d1} {dt} round 1", "+++ b/x")
    expect("diff_get full is the body verbatim", s.call("diff_get", id=d1, full=True), "--- a/x\n+++ b/x\n+one")
    s.call("task_update", id=dt, status="needs_fix")
    s.call("task_update", id=dt, status="working")
    second = s.call("diff_add", id=dt, body="+two")
    expect("a reopened task records round 2", second, f"({dt} round 2)")
    d2 = second.split()[0]
    listing = s.call("diff_list", id=dt).splitlines()
    ok = len(listing) == 2 and listing[0].startswith(f"{d2} round 2 ") and listing[1].startswith(f"{d1} round 1 ")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "diff_list is newest first :: " + " | ".join(listing))
    expect("task_get shows the newest diff line", s.call("task_get", id=dt), f"diff: {d2} round 2 ")
    expect("diff_add refuses a whitespace-only body", refusal_text(lambda: s.call("diff_add", id=dt, body=" \n ")), "the body is empty")
    ev_full = s.call("diff_add", id=dt, body="re-ran: cargo test ok", reviews=d2)
    ev = ev_full.split()[0]
    expect("evidence-only records a marked row against the newest patch", ev_full, f"({dt} round 2)")
    expect("diff_list marks the evidence-only row and names the patch", s.call("diff_list", id=dt), f"{ev} evidence-only -> {d2}")
    expect("diff_get labels an evidence-only row as not a diff", s.call("diff_get", id=ev, full=True), f"{ev} {dt} round 2 evidence-only, reviews {d2}", "not a diff", "re-ran: cargo test ok")
    expect("task_get still shows the patch, not the evidence row", s.call("task_get", id=dt), f"diff: {d2} round 2 ")
    expect("evidence-only refuses an empty body", refusal_text(lambda: s.call("diff_add", id=dt, body=" \n ", reviews=d2)), "the evidence is empty")
    expect("evidence-only refuses another evidence-only row", refusal_text(lambda: s.call("diff_add", id=dt, body="re-ran", reviews=ev)), f"{ev} is evidence-only, not a patch")
    expect("evidence-only refuses a superseded patch and names the newest", refusal_text(lambda: s.call("diff_add", id=dt, body="re-ran", reviews=d1)), f"{d1} is not the newest patch of {dt}", d2)
    other = s.call("task_add", title="another row").split()[0]
    outside = s.call("diff_add", id=other, body="other patch").split()[0]
    expect("evidence-only refuses a diff recorded on another row", refusal_text(lambda: s.call("diff_add", id=dt, body="re-ran", reviews=outside)), f"{outside} is recorded on {other}, not on {dt}")
    expect("evidence-only refuses a missing diff", refusal_text(lambda: s.call("diff_add", id=dt, body="re-ran", reviews="D9999")), "no such diff D9999")
    expect("diff_add refuses an unknown task", refusal_text(lambda: s.call("diff_add", id="T9999", body="x")), "no such task T9999")
    expect("diff_add refuses an epic id", refusal_text(lambda: s.call("diff_add", id=epic_id, body="x")), "is an epic id")
    expect("diff_get refuses a task id", refusal_text(lambda: s.call("diff_get", id=dt)), "not a diff id")
    expect("diff_get refuses an unknown diff", refusal_text(lambda: s.call("diff_get", id="D9999")), "no such diff D9999")
    expect("diff_list of an epic names each task", s.call("diff_list", id=epic_id), "(no diffs)")
    descs = {t["name"]: t.get("description", "") for t in s.rpc("tools/list", {})["tools"]}
    ok = all("newest" in descs[n] for n in ("diff_add", "diff_get", "diff_list"))
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "the three diff tool descriptions say only the newest diff is reviewed")

    # The review gate and the auto-filed review row.
    gp = s.call("task_add", title="gate probe", priority=9).split()[0]
    s.call("task_claim", id=gp)
    refused = refusal_text(lambda: s.call("task_update", id=gp, status="review", report="the report"))
    expect("a move to review with no diff is refused, naming diff_add and the task", refused, f"{gp} cannot move to review", f"diff add {gp}", f"diff_add id={gp}")
    expect("the refused move changed nothing", s.call("task_get", id=gp), f"{gp} working")
    dg = s.call("diff_add", id=gp, body="gate diff").split()[0]
    moved = s.call("task_update", id=gp, status="review", report="the report")
    expect("with a diff the same move succeeds and names the filed review row", moved, f"{gp} review #", "(review row T")
    rv = moved.split("(review row ")[1].rstrip(")")
    expect("task_children lists the one auto-filed review row", s.call("task_children", id=gp), rv, "Review gate probe")
    expect("the review row carries the report verbatim, the newest diff, the parent and the priority", s.call("task_get", id=rv, full=True), f"{rv} ready - priority 9", f"parent: {gp}", "the report", f"Newest diff: {dg}")
    s.call("task_update", id=gp, status="needs_fix")
    s.call("task_update", id=gp, status="working")
    expect("after a reopen and a new working move the old diff no longer satisfies the gate", refusal_text(lambda: s.call("task_update", id=gp, status="review")), f"diff add {gp}")
    d2 = s.call("diff_add", id=gp, body="gate diff two").split()[0]
    s.call("task_update", id=gp, status="review")
    kids = [l for l in s.call("task_children", id=gp).splitlines() if l.strip()]
    ok = len(kids) == 1
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + f"a resubmission files no second review row :: {len(kids)} child")
    expect("the existing review row is told the new diff", s.call("task_get", id=rv), f"{gp} submitted again: newest diff {d2} (round 2)")

    # A re-run round reaches review on evidence, and the brief names the patch to read.
    s.call("task_update", id=gp, status="needs_fix")
    s.call("task_update", id=gp, status="working")
    ev = s.call("diff_add", id=gp, body="re-ran: cargo test ok", reviews=d2).split()[0]
    moved = s.call("task_update", id=gp, status="review", report="re-ran the gates")
    expect("evidence-only satisfies the gate on a resubmission", moved, f"{gp} review #", f"(review row {rv})")
    expect("the resubmission note names the evidence row and the patch", s.call("task_get", id=rv), f"{gp} submitted again: no new diff; {ev} is evidence-only for {d2}", f"(round 2)", f"the diff under review is {d2}")
    # A row whose first submission is evidence-only: the review brief names that case.
    ep = s.call("task_add", title="evidence probe").split()[0]
    s.call("task_claim", id=ep)
    e1 = s.call("diff_add", id=ep, body="the first patch").split()[0]
    s.call("task_update", id=ep, status="needs_fix")
    s.call("task_update", id=ep, status="working")
    e2 = s.call("diff_add", id=ep, body="re-ran: all green", reviews=e1).split()[0]
    moved = s.call("task_update", id=ep, status="review", report="the evidence report")
    er = moved.split("(review row ")[1].rstrip(")")
    expect("the first evidence-only brief says so and copies the evidence verbatim", s.call("task_get", id=er, full=True), f"No new diff this round: {e2} is evidence-only for {e1}", f"the diff under review is {e1}", "Evidence (verbatim):", "all green", "evidence-only this round; it is not a patch", "the evidence report")

    catalog = {t["name"] for t in s.rpc("tools/list", {})["tools"]}
    wanted = {"task_add", "task_list", "task_get", "task_children", "task_claim", "task_update", "task_wait", "task_dispatch",
              "epic_add", "epic_list", "epic_get", "epic_update", "diff_add", "diff_get", "diff_list", "board_brief", "session_register"}
    checks.append(catalog == wanted)
    print(("PASS " if checks[-1] else "FAIL ") + f"router catalog is the retained surface: {len(catalog)} tools")
    for deleted in ["task_edit", "epic_edit", "task_ask", "session_list", "session_tail", "session_send", "board_build"]:
        expect(f"deleted {deleted} has no callable alias", refusal_text(lambda: s.call(deleted)), "-32602")
    for setter, obj in [("task_update", ed), ("epic_update", epic_id)]:
        expect(f"{setter} rejects unknown field by name", refusal_text(lambda: s.call(setter, id=obj, unknown_probe="no")), "unknown_probe")
    expect("spec update cannot request heartbeat", refusal_text(lambda: s.call("task_update", id=ed, title="must not land", heartbeat=True)), "spec edits cannot heartbeat")
    expect("refused spec/heartbeat leaves title intact", s.call("task_get", id=ed), "title: edited")
    combined = s.call("task_add", title="atomic before").split()[0]
    before = stable(s.call("task_get", id=combined, full=True))
    expect("mixed spec/state update refuses invalid status", refusal_text(lambda: s.call("task_update", id=combined, title="must roll back", status="invalid-probe")), "bad status")
    checks.append(stable(s.call("task_get", id=combined, full=True)) == before)
    print(("PASS " if checks[-1] else "FAIL ") + "failed combined task update preserves spec and events")
    expect("combined task setter updates spec and state", s.call("task_update", id=combined, title="atomic after", status="blocking", note="human decides"), f"{combined} blocking")
    expect("combined task fields survive", s.call("task_get", id=combined), "title: atomic after", "human decides")
    both_epic = s.call("epic_add", title="atomic epic").split()[0]
    expect("combined epic setter updates spec and state", s.call("epic_update", id=both_epic, title="renamed epic", status="done"), f"{both_epic} edited title, status")
    expect("combined epic fields survive", s.call("epic_get", id=both_epic), "title: renamed epic", f"{both_epic} done")
    before = stable(s.call("epic_get", id=both_epic, full=True))
    expect("mixed epic setter refuses invalid status", refusal_text(lambda: s.call("epic_update", id=both_epic, title="bad edit", status="invalid-probe")), "bad epic status")
    checks.append(stable(s.call("epic_get", id=both_epic, full=True)) == before)
    print(("PASS " if checks[-1] else "FAIL ") + "failed combined epic update preserves spec and events")

    # Creation actor is immutable, shared by CLI/MCP rendering, never an argument.
    analyzer = Session("analyzer")
    coder = Session("coder")
    try:
        analyzed = analyzer.call("task_add", title="analyzed").split()[0]
        implemented = coder.call("task_add", title="implemented").split()[0]
        expect("reporter tracks analyzer creation", s.call("task_get", id=analyzed), "reporter: analyzer")
        expect("reporter tracks coder creation", s.call("task_get", id=implemented), "reporter: coder")
        coder.call("task_update", id=analyzed, title="renamed analyzed")
        expect("reporter survives another actor's edit", s.call("task_get", id=analyzed), "reporter: analyzer", "title: renamed analyzed")
        expect("MCP add refuses reporter spoof", refusal_text(lambda: coder.call("task_add", title="forged", reporter="fake")), "reporter")
        expect("MCP setter refuses reporter overwrite", refusal_text(lambda: coder.call("task_update", id=analyzed, reporter="fake")), "reporter")
        result = subprocess.run([BIN, "--db", DB, "get", analyzed, "--full"], capture_output=True, text=True, timeout=20)
        expect("CLI and MCP show identical reporter", result.stdout if result.returncode == 0 else result.stderr, "reporter: analyzer")
    finally:
        analyzer.close()
        coder.close()

    # The coder files one review child, guarded by identity, current diff and plan.
    coder = Session("gate-coder")
    reviewer = Session("gate-reviewer")
    try:
        plan = s.call("task_add", title="gate analyze").split()[0]
        s.call("task_update", id=plan, status="needs_impl", report="the agreed plan")
        impl = s.call("task_add", title="gate implement", parent=plan).split()[0]
        coder.call("task_claim", id=impl)
        expect("review creation refuses absent diff", refusal_text(lambda: coder.call("task_add", title="review", parent=impl, description=f"D1 {plan}")), "no diff", "since it was last claimed")
        patch = coder.call("diff_add", id=impl, body="complete patch one").split()[0]
        expect("review creation refuses absent plan", refusal_text(lambda: coder.call("task_add", title="review", parent=impl, description=f"Read {patch}")), "description must name diff", f"analyze plan {plan}")
        expect("review creation refuses absent diff reference", refusal_text(lambda: coder.call("task_add", title="review", parent=impl, description=f"Read {plan}")), f"diff {patch}")
        for other in [s, reviewer]:
            expect("only implement holder files review", refusal_text(lambda: other.call("task_add", title="review", parent=impl, description=f"Read {patch} against {plan}")), "reporter", impl)
        review = coder.call("task_add", title="review", parent=impl, description=f"Read {patch} against {plan}").split()[0]
        expect("filing review publishes implement atomically", s.call("task_get", id=impl), f"{impl} needs_review")
        expect("review records coder as reporter", s.call("task_get", id=review), "reporter: gate-coder")
        reviewer.call("task_claim", id=review)
        reviewer.call("task_update", id=review, status="needs_fix", report="blocker: correct the patch")
        expect("review rejection reopens implement in one step", s.call("task_get", id=impl), f"{impl} needs_fix", "reopens: 1/3")
        coder.call("task_claim", id=impl)
        expect("review gate refuses old round diff", refusal_text(lambda: coder.call("task_update", id=impl, status="needs_review")), "no diff")
        patch2 = coder.call("diff_add", id=impl, body="complete patch two").split()[0]
        coder.call("task_update", id=impl, status="needs_review", report="corrected")
        expect("resubmission refreshes same review brief", s.call("task_get", id=review, full=True), f"{review} ready", patch2, f"Analyze plan {plan}")
        expect("diff history preserves both rounds", s.call("diff_list", id=impl), patch, patch2, "round 1", "round 2")
        expect("resubmission refuses a second review row", refusal_text(lambda: coder.call("task_add", title="review again", parent=impl, description=f"{patch2} {plan}")), "only one child")
    finally:
        coder.close()
        reviewer.close()

    # Change the board stamp behind an already initialized MCP process. The
    # next write must fail, even on SQLite where initialization used to be cached.
    if BACKEND == "sqlite":
        import sqlite3
        with sqlite3.connect(DB) as c:
            c.execute("UPDATE board_stamp SET generation=99")
    else:
        import urllib.request
        req = urllib.request.Request("http://" + URL.removeprefix("http://") + "/sql",
            data=b"UPDATE meta:schema SET version=99",
            headers={"Authorization": "Basic cm9vdDpyb290", "surreal-ns": "agentboard",
                     "surreal-db": "board", "Accept": "application/json", "Content-Type": "text/plain"})
        with urllib.request.urlopen(req) as response:
            response.read()
    refused = refusal_text(lambda: s.call("task_add", title="surface-refused"))
    expect("future schema refusal has MCP -32603 and named remedy", refused,
           "-32603", "binary is older than the board", "use a newer binary")
    expect("schema refusal names board and binary versions", refused,
           "schema version 99", "understands version 3")
    s.close()
    print(f"\n{sum(checks)}/{len(checks)} checks passed (backend: {BACKEND})")
    return 0 if all(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
