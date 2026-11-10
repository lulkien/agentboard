#!/usr/bin/env python3
"""Daemon check: `agentboard daemon` on scratch boards and loopback ports.

Every daemon binds 127.0.0.1:0 and reports its port on the startup line; nothing
here touches a live board or a fixed port.
"""

import http.client
import json
import os
import subprocess
import sys
import tempfile
import urllib.parse

BIN = os.path.abspath(os.environ.get("AGENTBOARD_TEST_BIN", "target/debug/agentboard"))
PROTOCOL = "2025-11-25"
passed = 0
failed = 0


def check(name, want, got):
    global passed, failed
    if want == got:
        passed += 1
    else:
        failed += 1
        print(f"FAIL: {name}: want {want!r} got {got!r}")


def has(name, needle, hay):
    global passed, failed
    if needle in hay:
        passed += 1
    else:
        failed += 1
        print(f"FAIL: {name}: missing {needle!r} in {hay[:400]!r}")


class Daemon:
    def __init__(self, db, extra_env=None, cwd=None, address="127.0.0.1:0"):
        env = {k: v for k, v in os.environ.items() if not k.startswith("AGENTBOARD_")}
        env.update(extra_env or {})
        self.proc = subprocess.Popen(
            [BIN, "--db", db, "daemon", address],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True, cwd=cwd,
        )
        self.line = self.proc.stdout.readline()
        assert "listening on http://" in self.line, (self.line, self.proc.stderr.read())
        self.host, port = self.line.split("http://")[1].split()[0].rsplit(":", 1)
        self.port = int(port)
        self.seq = 0

    def request(self, method, path, body=None, headers=None):
        conn = http.client.HTTPConnection(self.host, self.port, timeout=30)
        conn.request(method, path, body, headers or {})
        response = conn.getresponse()
        data = response.read().decode()
        conn.close()
        return response.status, data

    def rpc(self, path, method, params=None, headers=None):
        self.seq += 1
        message = {"jsonrpc": "2.0", "id": self.seq, "method": method, "params": params or {}}
        head = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
        head.update(headers or {})
        return self.request("POST", path, json.dumps(message), head)

    def call(self, path, tool, arguments=None):
        status, body = self.rpc(path, "tools/call", {"name": tool, "arguments": arguments or {}})
        assert status == 200, (status, body)
        result = json.loads(body)["result"]
        return "".join(part.get("text", "") for part in result["content"])

    def close(self):
        self.proc.kill()
        self.proc.wait()
        self.proc.stdout.close()
        self.proc.stderr.close()


def cli(db, *args, **kwargs):
    return subprocess.run([BIN, "--db", db, *args], capture_output=True, text=True, **kwargs)


def main():
    if not os.path.exists(BIN):
        raise SystemExit(f"no binary at {BIN}; cargo build first")
    scratch = tempfile.mkdtemp(prefix="agentboard-daemon.")
    db = os.path.join(scratch, "board")

    # Startup line, health, and the bind refusal.
    daemon = Daemon(db)
    try:
        has("startup line names the board", db, daemon.line)
        status, body = daemon.request("GET", "/api/health")
        health = json.loads(body)
        check("health is 200", 200, status)
        check("health names the resolved board", db, health["board"])
        has("startup line names the instance", health["instance"], daemon.line)
        check("health refuses POST", 405, daemon.request("POST", "/api/health", "{}")[0])
        check("unknown path is 404", 404, daemon.request("GET", "/nope")[0])

        # Identity is the URL segment: two seats interleaved on one daemon.
        daemon.rpc("/mcp/alice", "initialize", {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": {"name": "c", "version": "0"}})
        daemon.call("/mcp/alice", "task_add", {"title": "from alice"})
        daemon.call("/mcp/bob", "task_add", {"title": "from bob"})
        daemon.call("/mcp/alice", "task_comment", {"id": 2, "body": "alice on bob's row"})
        daemon.call("/mcp/bob", "task_comment", {"id": 1, "body": "bob on alice's row"})
        feed = daemon.call("/mcp/alice", "board_feed", {"after": "0"})
        lines = {line.split()[2] + " " + " ".join(line.split()[3:5]) for line in feed.splitlines() if " created " in line or " commented " in line}
        has("alice's add is attributed to alice", "alice created T1", " ".join(lines))
        has("bob's add is attributed to bob", "bob created T2", " ".join(lines))
        has("alice's comment is attributed to alice", "alice commented T2", " ".join(lines))
        has("bob's comment is attributed to bob", "bob commented T1", " ".join(lines))
        agents = daemon.call("/mcp/alice", "board_agents")
        has("roster has alice", "alice", agents)
        has("roster has bob", "bob", agents)
        escaped = urllib.parse.quote("al ice<&", safe="")
        daemon.call("/mcp/" + escaped, "task_add", {"title": "odd name"})
        has("the actor segment is percent-decoded", "al ice<&", daemon.call("/mcp/alice", "board_agents"))

        # Guard.
        check("/mcp/ with no name is 404", 404, daemon.rpc("/mcp/", "tools/list")[0])
        check("/mcp with no slash is 404", 404, daemon.rpc("/mcp", "tools/list")[0])
        check("foreign Origin is 403", 403, daemon.rpc("/mcp/alice", "tools/list", headers={"Origin": "http://evil.example"})[0])
        check("Origin null is 403", 403, daemon.rpc("/mcp/alice", "tools/list", headers={"Origin": "null"})[0])
        check("foreign Origin is refused on the API too", 403, daemon.request("GET", "/api/health", headers={"Origin": "http://evil.example"})[0])
        for origin in ["http://localhost", "http://127.0.0.1", "http://localhost:80"]:
            check(f"Origin {origin} (port 80) is 403 on another port", 403, daemon.rpc("/mcp/alice", "tools/list", headers={"Origin": origin})[0])
        check("our own Origin passes", 200, daemon.request("GET", "/api/health", headers={"Origin": f"http://127.0.0.1:{daemon.port}"})[0])
        check("foreign Host is 421", 421, daemon.rpc("/mcp/alice", "tools/list", headers={"Host": "evil.example"})[0])
        check("foreign Host with our port is 421", 421, daemon.rpc("/mcp/alice", "tools/list", headers={"Host": f"evil.example:{daemon.port}"})[0])
        check("localhost Host passes", 200, daemon.request("GET", "/api/health", headers={"Host": f"localhost:{daemon.port}"})[0])
        rejected = daemon.rpc("/mcp/mallory", "tools/call", {"name": "task_add", "arguments": {"title": "evil"}}, headers={"Host": "evil.example"})[0]
        check("a foreign Host cannot write", 421, rejected)
        check("a foreign Host wrote nothing", False, "mallory" in daemon.call("/mcp/alice", "board_agents"))
        check("a bad project is 400", 400, daemon.rpc("/mcp/alice?project=x", "tools/list")[0])
        check("a repeated project is 400", 400, daemon.rpc("/mcp/alice?project=1&project=1", "tools/list")[0])

        # reset against a resident daemon is the busy refusal.
        reset = cli(db, "reset", "--yes")
        check("reset against the daemon's board exits 1", 1, reset.returncode)
        has("reset names the busy refusal", "another client is attached", reset.stderr)
        check("the board survived the reset", 3, int(daemon.call("/mcp/alice", "task_list").split("total ")[1].split()[0]))
    finally:
        daemon.close()

    # Non-loopback bind needs the flag.
    refused = subprocess.run([BIN, "--db", db, "daemon", "0.0.0.0:0"], capture_output=True, text=True, timeout=30)
    check("non-loopback bind without the flag exits 1", 1, refused.returncode)
    has("non-loopback refusal names the flag", "--allow-non-loopback", refused.stderr)

    # Project scope: argument, else the URL, never the daemon's env or cwd.
    pdb = os.path.join(scratch, "projects")
    here = os.path.join(scratch, "cwd")
    os.makedirs(here)
    cli(pdb, "project", "add", "--name", "A", "--path", "/project-a")
    cli(pdb, "project", "add", "--name", "B", "--path", here)
    cli(pdb, "--actor", "seed", "add", "A row", "--project", "1")
    cli(pdb, "--actor", "seed", "add", "B row", "--project", "2")
    cli(pdb, "--actor", "seed", "add", "loose row")
    daemon = Daemon(pdb, extra_env={"AGENTBOARD_PROJECT": "2"}, cwd=here)
    try:
        listing = daemon.call("/mcp/w?project=1", "task_list")
        has("?project= scopes task_list", "A row", listing)
        check("?project= excludes the other project", False, "B row" in listing)
        check("?project= excludes unscoped rows", False, "loose row" in listing)
        listing = daemon.call("/mcp/w?project=1", "task_list", {"project": 2})
        has("a tool argument beats the URL project", "B row", listing)
        check("a tool argument replaces the URL project", False, "A row" in listing)
        brief = daemon.call("/mcp/w?project=1", "board_brief")
        has("?project= scopes board_brief", "todo 1 ", brief)
        has("the unscoped brief counts the whole board", "todo 3 ", daemon.call("/mcp/w", "board_brief"))
        listing = daemon.call("/mcp/w", "task_list")
        for row in ["A row", "B row", "loose row"]:
            has(f"no project on the URL lists the whole board ({row})", row, listing)
        print("project scope: argument, URL, then whole board; daemon env and cwd ignored")
    finally:
        daemon.close()

    print(f"daemon: {passed}/{passed + failed} passed (http, lmdb)")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
