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
    expect("heartbeat", s.call("task_update", id="T1", heartbeat=True), "lease")
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

    expect("session list", s.call("session_list"), "session")
    s.close()
    print(f"\n{sum(checks)}/{len(checks)} checks passed")
    return 0 if all(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
