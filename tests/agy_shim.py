#!/usr/bin/env python3
"""agy stdio shim: JSON-RPC lines in, the daemon's /mcp out, actor from AGENTBOARD_ACTOR.

A scratch daemon on loopback port 0 and a temp board; nothing touches a live board.
"""
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SHIM = os.path.join(ROOT, "seats/agy/agentboard-stdio")
BIN = os.path.abspath(os.environ.get("AGENTBOARD_TEST_BIN") or os.path.join(ROOT, "target/release/agentboard"))
passed = failed = 0


def check(name, condition, detail=""):
    global passed, failed
    if condition:
        passed += 1
    else:
        failed += 1
        print(f"FAIL: {name} {detail}")


def run_shim(lines, env_extra, drop=()):
    env = {k: v for k, v in os.environ.items() if not k.startswith("AGENTBOARD_") and k not in drop}
    env.update(env_extra)
    done = subprocess.run([SHIM], input="".join(json.dumps(x) + "\n" for x in lines),
                          capture_output=True, text=True, env=env, timeout=60)
    return done, [json.loads(x) for x in done.stdout.splitlines()]


def main():
    db = os.path.join(tempfile.mkdtemp(), "b")
    env = {k: v for k, v in os.environ.items() if not k.startswith("AGENTBOARD_")}
    board = lambda *a: subprocess.run([BIN, "--db", db, "--actor", "manager", *a], capture_output=True, text=True, env=env)
    check("scratch task added", board("add", "shim target", "--project", "0").returncode == 0)
    daemon = subprocess.Popen([BIN, "--db", db, "daemon", "127.0.0.1:0"], stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, text=True, env=env)
    try:
        line = daemon.stdout.readline()
        assert "listening on http://" in line, (line, daemon.stderr.read())
        url = "http://" + line.split("http://")[1].split()[0] + "/mcp"
        init = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}}
        note = {"jsonrpc": "2.0", "method": "notifications/initialized"}
        call = {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": "task_comment", "arguments": {"id": 1, "body": "from the shim"}}}
        done, out = run_shim([init, note, call], {"AGENTBOARD_ACTOR": "analyst", "AGENTBOARD_URL": url})
        check("shim exits 0 at end of input", done.returncode == 0, done.stderr)
        check("one reply per request, none for the notification", [o.get("id") for o in out] == [1, 2], out)
        check("initialize answered", "result" in out[0], out[:1])
        feed = board("feed", "--after", "0", "--full").stdout
        check("comment author is analyst", any(
            "analyst" in row and "commented" in row and "from the shim" in row for row in feed.splitlines()), feed)

        done, out = run_shim([call], {"AGENTBOARD_ACTOR": "${AGENTBOARD_ACTOR}", "AGENTBOARD_URL": url})
        check("a daemon refusal comes back as an error carrying its message",
              len(out) == 1 and out[0]["id"] == 2 and "HTTP 400" in out[0]["error"]["message"]
              and "AGENTBOARD_ACTOR" in out[0]["error"]["message"], out)

        done, out = run_shim([init], {}, drop=("AGENTBOARD_ACTOR",))
        check("unset actor exits 2", done.returncode == 2, done.returncode)
        check("unset actor says why", "agentboard-stdio: set AGENTBOARD_ACTOR" in done.stderr, done.stderr)
        done, _ = run_shim([init], {"AGENTBOARD_ACTOR": ""})
        check("empty actor exits 2", done.returncode == 2, done.returncode)
    finally:
        daemon.terminate()
        daemon.wait()
    done, out = run_shim([call], {"AGENTBOARD_ACTOR": "analyst", "AGENTBOARD_URL": url})
    check("daemon down gives a JSON-RPC error with the same id",
          done.returncode == 0 and len(out) == 1 and out[0]["id"] == 2 and "error" in out[0], (done.stdout, done.stderr))
    print(f"agy-shim: {passed}/{passed + failed} passed")
    return 1 if failed else 0


sys.exit(main())
