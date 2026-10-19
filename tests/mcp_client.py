#!/usr/bin/env python3
"""Sequential MCP client: drives the real binary over stdio, one request at a
time (what a real session does), and asserts on the text each tool returns."""
import json
import os
import shutil
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target/release/agentboard")
DB = os.path.join(ROOT, ".probe/client.db")


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
        return r["content"][0]["text"]

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=5)


def main():
    shutil.rmtree(os.path.dirname(DB), ignore_errors=True)
    s = Session()
    checks = []

    def expect(label, got, *needles):
        ok = all(n in got for n in needles)
        checks.append(ok)
        print(("PASS " if ok else "FAIL ") + label + " :: " + got.replace("\n", " | ")[:120])

    expect("add", s.call("task_add", title="write fd docs", description="one page, SCM_RIGHTS", goal="verification", assignee="claude1", priority=5), "T1 ready")
    expect("list", s.call("task_list"), "T1 rdy")
    expect("claim returns description+goal", s.call("task_claim", id="T1", ttl_min=5), "T1 working", "one page, SCM_RIGHTS", "verification")
    expect("claim twice", s.call("task_claim", id="T1"), "T1 working by claude1")
    expect("note", s.call("task_update", id="T1", note="outline done"), "T1 working #")
    expect("note does not extend lease", s.call("task_get", id="T1"), "lease: 4m left")
    expect("heartbeat", s.call("task_update", id="T1", heartbeat=True), "lease")
    expect("heartbeat with note", s.call("task_update", id="T1", heartbeat=True, ttl_min=60, note="still going"), "T1 working #")
    expect("heartbeat applied alongside note", s.call("task_get", id="T1"), "lease: 59m left")
    intruder = Session(actor="intruder")
    expect("claim by other while leased", intruder.call("task_claim", id="T1"), "not claimable")
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
    expect("ask", s.call("task_ask", id="T2", question="nix or libc?"), "T2 blocking", "#")

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
        ok = "no such task" in str(e)
        checks.append(ok)
        print(("PASS " if ok else "FAIL ") + f"claim missing task -> {e}")

    try:
        s.call("task_add", title="orphan", parent="T99")
        expect("missing parent rejected", "accepted")
    except RuntimeError as e:
        expect("missing parent rejected", str(e), "no such parent task T99")
    s.call("task_add", title="low", priority=50)
    s.call("task_add", title="high", priority=1)
    expect("claim next honours priority", s.call("task_claim"), "high")

    # Epic workflow: parent roundtrip, needs_fix (+ alias), children, rollup.
    expect("add epic", s.call("task_add", title="epic: ship it", description="epic"), "ready")
    epic = s.call("task_list", limit=50)
    epic_id = [l.split()[0] for l in epic.splitlines() if "epic: ship it" in l][0]
    reply = s.call("task_add", title="implement it", parent=epic_id)
    expect("add child with parent", reply, "ready")
    child_id = reply.split()[0]
    expect("parent roundtrip on get", s.call("task_get", id=child_id), f"parent: {epic_id}")
    review_id = s.call("task_add", title="review it", parent=epic_id).split()[0]
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
    for bad in ("T9999", "nonsense", ""):
        try:
            s.call("task_children", id=bad)
            expect(f"children bad id {bad!r} errors", "accepted")
        except RuntimeError as e:
            checks.append(True)
            print(f"PASS children bad id {bad!r} errors :: {e}")
    expect("needs_fix claimable only via lease msg", s.call("task_claim", id=child_id), f"{child_id} needs_fix", "not claimable")
    ok = "(lease)" not in s.call("task_claim", id=child_id)
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "no (lease) text for lease-less needs_fix")
    for alias in ("needs-fix", "fix", "Needs Fix"):
        s.call("task_update", id=child_id, status="working")
        expect(f"alias {alias!r}", s.call("task_update", id=child_id, status=alias), f"{child_id} needs_fix")
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
    brief = s.call("board_brief").splitlines()
    bidx = {l.split()[0]: i for i, l in enumerate(brief) if l.split()}
    ok = bidx[child_id] < bidx[review_id]
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "brief orders needs_fix above working")
    s.call("task_update", id=review_id, status="ready")
    expect("get shows children line", s.call("task_get", id=epic_id), "children: 2 (1 needs_fix, 1 ready)")
    expect("brief shows epic line", s.call("board_brief"), f"EPIC {epic_id} 0/2 done epic: ship it")

    # Dropped children leave the denominator; closed epics drop out; nesting is one level.
    dropped_id = s.call("task_add", title="abandoned", parent=epic_id).split()[0]
    s.call("task_update", id=dropped_id, status="dropped")
    done_id = s.call("task_add", title="finished", parent=epic_id).split()[0]
    s.call("task_update", id=done_id, status="done")
    expect("rollup excludes dropped", s.call("board_brief"), f"EPIC {epic_id} 1/3 done epic: ship it")
    sub_id = s.call("task_add", title="sub-epic", parent=epic_id).split()[0]
    s.call("task_add", title="grandchild", parent=sub_id)
    brief = s.call("board_brief")
    expect("nested epic rolls up one level", brief, f"EPIC {epic_id} 1/4 done", f"EPIC {sub_id} 0/1 done")
    s.call("task_update", id=epic_id, status="done")
    ok = f"EPIC {epic_id} " not in s.call("board_brief")
    checks.append(ok)
    print(("PASS " if ok else "FAIL ") + "closed epic has no EPIC line")

    expect("session list", s.call("session_list"), "session")
    s.close()
    print(f"\n{sum(checks)}/{len(checks)} checks passed")
    return 0 if all(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
