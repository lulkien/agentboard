#!/usr/bin/env python3
"""MCP surface check: nine tools on one stdio server, no daemon, scratch board."""

import json
import os
import subprocess
import sys
import tempfile

BIN = os.environ.get("AGENTBOARD_TEST_BIN", os.path.join(os.getcwd(), "target/debug/agentboard"))
PROTOCOL = "2025-11-25"
TOOLS = {
    "task_add",
    "task_list",
    "task_get",
    "task_update",
    "task_claim",
    "task_comment",
    "board_feed",
    "board_brief",
    "board_agents",
}

passed = 0
failed = 0


def ok(name):
    global passed
    passed += 1


def bad(name, detail):
    global failed
    failed += 1
    print(f"FAIL: {name}: {detail}")


def check(name, want, got):
    if want == got:
        ok(name)
    else:
        bad(name, f"want {want!r} got {got!r}")


def has(name, needle, hay):
    if needle in hay:
        ok(name)
    else:
        bad(name, f"missing {needle!r} in {hay[:400]!r}")


class Server:
    def __init__(self, db, actor, args=()):
        env = dict(os.environ, AGENTBOARD_DB=db, AGENTBOARD_ACTOR=actor)
        self.proc = subprocess.Popen(
            [BIN, "serve", *args],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            text=True,
            bufsize=1,
        )
        self.seq = 0

    def send(self, method, params=None, notification=False):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if not notification:
            self.seq += 1
            message["id"] = self.seq
        self.proc.stdin.write(json.dumps(message) + "\n")
        self.proc.stdin.flush()
        if notification:
            return None
        line = self.proc.stdout.readline()
        if not line:
            raise SystemExit(f"server closed the stream: {self.proc.stderr.read()}")
        return json.loads(line)

    def call(self, tool, arguments=None):
        reply = self.send("tools/call", {"name": tool, "arguments": arguments or {}})
        if "error" in reply:
            return True, json.dumps(reply["error"])
        result = reply["result"]
        body = "".join(part.get("text", "") for part in result.get("content", []))
        return bool(result.get("isError")), body

    def start(self):
        hello = self.send(
            "initialize",
            {
                "protocolVersion": PROTOCOL,
                "capabilities": {},
                "clientInfo": {"name": "check", "version": "0"},
            },
        )
        self.send("notifications/initialized", {}, notification=True)
        return hello

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()
        return False


def main():
    if not os.path.exists(BIN):
        raise SystemExit(f"no binary at {BIN}; cargo build first")
    scratch = tempfile.mkdtemp(prefix="agentboard-mcp.")
    server = Server(os.path.join(scratch, "board"), "worker")
    try:
        hello = server.start()
        has("initialize names the server", "agentboard", json.dumps(hello))

        listed = server.send("tools/list", {})
        names = {tool["name"] for tool in listed["result"]["tools"]}
        check("tools/list is the nine-tool surface", TOOLS, names)

        is_error, body = server.call("task_add", {"title": "mcp task", "description": "full text here"})
        check("task_add is not an error", False, is_error)
        has("task_add returns the record", "T1 todo", body)
        has("task_add returns a cursor", "cursor 1", body)

        is_error, body = server.call("task_list")
        check("task_list is not an error", False, is_error)
        has("task_list lists the row", "T1", body)
        has("task_list totals", "total 1", body)

        is_error, body = server.call("task_get", {"id": 1, "full": True})
        has("task_get prints full text", "full text here", body)
        is_error, body = server.call("task_get", {"id": 1})
        has("task_get prints a revision", "revision 1", body)

        is_error, body = server.call("task_claim", {"id": 1})
        check("task_claim is not an error", False, is_error)
        has("task_claim returns a work order", "in_progress", body)
        is_error, body = server.call("task_claim")
        has("an empty claim answers no work", "no ready tasks", body)

        is_error, body = server.call("task_comment", {"id": 1, "body": "notes from mcp"})
        has("task_comment acknowledges", "ok", body)

        is_error, body = server.call("board_feed", {"kind": "commented"})
        has("board_feed finds the comment", "notes from mcp", body)
        has("board_feed prints a cursor", "cursor ", body)

        is_error, body = server.call("board_brief", {})
        has("board_brief counts in_progress", "in_progress 1", body)

        is_error, body = server.call("board_agents", {})
        has("board_agents lists the actor", "worker", body)

        is_error, body = server.call("task_update", {"id": 1, "if_revision": 1, "title": "stale"})
        check("stale revision is an error", True, is_error)
        has("stale revision names the numbers", "stale task revision 1", body)

        is_error, body = server.call("task_update", {"id": 1, "if_revision": 2, "force": True, "title": "x"})
        check("force without reason is an error", True, is_error)
        has("force without reason asks for one", "reason", body)

        is_error, body = server.call("board_nope", {})
        check("an unknown tool is an error", True, is_error)


        # Long text: compact reads cut it, full reads return it whole.
        long_comment = "c" * 400
        long_title = "t" * 100
        is_error, body = server.call("task_comment", {"id": 1, "body": long_comment})
        check("long comment accepted", False, is_error)
        is_error, body = server.call("board_feed", {"kind": "commented"})
        has("compact feed marks the cut", "…", body)
        check("compact feed hides the whole comment", False, long_comment in body)
        is_error, body = server.call("board_feed", {"kind": "commented", "full": True})
        has("full feed returns the whole comment", long_comment, body)

        is_error, body = server.call("task_add", {"title": long_title})
        has("long title accepted", "ok", body)
        _, body = server.call("task_get", {"id": 2})
        has("compact task_get truncates the title", "…", body)
        _, body = server.call("task_get", {"id": 2, "full": True})
        has("full task_get returns the whole title", long_title, body)

        # A comment with no field change is a comment; claiming your own todo starts it.
        is_error, body = server.call("task_add", {"title": "own todo", "assignee": "worker", "new_actor": True})
        has("assigned todo created", "ok", body)
        is_error, body = server.call("task_claim", {"id": 3})
        has("claiming your assigned todo starts it", "in_progress", body)
        has("claiming your assigned todo bumps the revision", "revision 2", body)
        is_error, body = server.call("task_update", {"id": 3, "if_revision": 2, "comment": "only a note"})
        check("comment-only update is not an error", False, is_error)
        has("comment-only update keeps the revision", "revision 2", body)
        is_error, body = server.call("board_feed", {"kind": "commented"})
        has("comment-only update lands in the feed", "only a note", body)


        is_error, body = server.call(
            "task_update", {"id": 1, "if_revision": 2, "assignee": "bob", "clear_assignee": True}
        )
        check("assignee with clear_assignee is an error", True, is_error)
        has("assignee conflict names the choice", "not both", body)

        # --actor beats the exported variable, so a wrapper flag is not ignored.
        with Server(os.path.join(scratch, "board"), "alice", args=["--actor", "bob"]) as flag_server:
            flag_server.start()
            _, body = flag_server.call("task_add", {"title": "flagged actor"})
            has("flag actor writes the task", "ok", body)
            _, body = flag_server.call("board_agents", {})
            has("flag actor registers bob", "bob", body)
            check("flag actor wins over the environment", False, "alice" in body)
    finally:
        server.close()

    print(f"{passed}/{passed + failed} passed (mcp, lmdb)")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
