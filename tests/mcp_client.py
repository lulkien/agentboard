#!/usr/bin/env python3
"""MCP surface check: eleven tools on one stdio server, no daemon, scratch board."""

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
    "task_epic",
    "task_update",
    "task_claim",
    "task_comment",
    "task_wait",
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
            [BIN, "--db", db, "serve", *args],
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
        check("tools/list is the eleven-tool surface", TOOLS, names)

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
        # CLI and MCP share one snapshot and renderer, with no intervening writes.
        with Server(os.path.join(scratch, "epics"), "worker") as epics:
            epics.start()
            _, body = epics.call("task_add", {"title": "umbrella", "priority": 20})
            for status in ["todo", "done", "canceled"]:
                epics.call("task_add", {"title": status + " child", "parent": 1, "status": status})
            epics.call("task_add", {"title": "ordinary", "status": "review", "priority": -1})
            error, body = epics.call("task_epic", {"id": 1})
            check("task_epic succeeds", False, error)
            check("epic four lines", 4, len(body.splitlines()))
            check("epic direct counts", "todo 1  in_progress 0  blocked 0  review 0  done 1  canceled 1", body.splitlines()[1])
            check("epic progress", "done 1/3", body.splitlines()[2])
            has("epic total cursor", "total 3 cursor ", body)
            cli = subprocess.check_output([BIN, "--db", os.path.join(scratch, "epics"), "epic", "1"], text=True)
            check("CLI and MCP epic bytes match", cli.removesuffix("\n"), body)
            print("epic parity: CLI == MCP (four lines, same scratch snapshot)")
            for ident, message in [(2, "task T2 is not an epic; an epic is a task with children"), (999, "task T999 does not exist")]:
                reply = epics.send("tools/call", {"name": "task_epic", "arguments": {"id": ident}})
                check("epic read refusal code", -32602, reply.get("error", {}).get("code"))
                check("epic read refusal text", message, reply.get("error", {}).get("message"))
            for status in ["done", "canceled"]:
                reply = epics.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 1, "status": status}})
                check("epic close JSON-RPC code", -32602, reply.get("error", {}).get("code"))
                check("epic close exact count", "epic T1 has 1 open children; close or drop them first", reply.get("error", {}).get("message"))
            print("epic refusal: -32602 epic T1 has 1 open children; close or drop them first")
            _, brief = epics.call("board_brief")
            check("brief epic first", "T1", brief.splitlines()[0].split()[0])
            has("brief progress", "done 1/3", brief.splitlines()[0])
            check("brief ordinary second", "T5", brief.splitlines()[1].split()[0])
            check("brief excludes children", 3, len(brief.splitlines()))
            check("brief global counts last", "todo 2  in_progress 0  blocked 0  review 1  done 1  canceled 1", brief.splitlines()[-1])
            epics.call("task_update", {"id": 2, "if_revision": 1, "status": "canceled"})
            error, body = epics.call("task_update", {"id": 1, "if_revision": 1, "status": "done"})
            check("all terminal epic closes", False, error)
            has("closed epic body", "T1 done", body)
            print("tools/list: task_epic present")
        for status in ["done", "canceled"]:
            with Server(os.path.join(scratch, "parent-" + status), "manager") as parents:
                parents.start()
                parents.call("task_add", {"title": "parent", "status": status})
                parents.call("task_add", {"title": "move"})
                _, before = parents.call("task_list", {"all": True})
                _, parent_before = parents.call("task_get", {"id": 1})
                _, child_before = parents.call("task_get", {"id": 2})
                for tool, args in [
                    ("task_add", {"title": "new", "parent": 1}),
                    ("task_update", {"id": 2, "if_revision": 1, "parent": 1}),
                ]:
                    reply = parents.send("tools/call", {"name": tool, "arguments": args})
                    check("terminal parent code", -32602, reply.get("error", {}).get("code"))
                    check("terminal parent exact message", "parent T1 is " + status + "; reopen it before adding children", reply.get("error", {}).get("message"))
                    _, after = parents.call("task_list", {"all": True})
                    check("terminal refusal tasks and cursor unchanged", before, after)
                    _, after = parents.call("task_get", {"id": 1})
                    check("parent not auto-reopened", parent_before, after)
                    _, after = parents.call("task_get", {"id": 2})
                    check("moved child unchanged", child_before, after)
                error, _ = parents.call("task_update", {"id": 1, "if_revision": 1, "status": "todo"})
                check("manager reopens parent", False, error)
                error, body = parents.call("task_add", {"title": "new", "parent": 1})
                check("reopened add succeeds", False, error)
                has("reopened add link", "parent T1", body)
                error, body = parents.call("task_update", {"id": 2, "if_revision": 1, "parent": 1})
                check("reopened reparent succeeds", False, error)
                has("reopened reparent link", "parent T1", body)

        for status in ["done", "canceled"]:
            with Server(os.path.join(scratch, "closed-" + status), "manager") as closed:
                closed.start()
                closed.call("task_add", {"title": "held", "status": status, "assignee": "alice", "new_actor": True})
                _, task_before = closed.call("task_get", {"id": 1})
                _, feed_before = closed.call("board_feed", {"after": "0"})
                text = status + " task cannot change assignee; reopen it with an open status in the same update"
                for args in [
                    {"assignee": "coder", "new_actor": True},
                    {"clear_assignee": True},
                    {"assignee": "coder", "new_actor": True, "force": True, "reason": "quota"},
                ]:
                    reply = closed.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 1, **args}})
                    check("closed owner change code", -32602, reply.get("error", {}).get("code"))
                    check("closed owner change message", text, reply.get("error", {}).get("message"))
                    check("closed owner change task unchanged", task_before, closed.call("task_get", {"id": 1})[1])
                    check("closed owner change no event", feed_before, closed.call("board_feed", {"after": "0"})[1])
                error, body = closed.call("task_update", {"id": 1, "if_revision": 1, "status": "todo", "assignee": "coder", "new_actor": True})
                check("reopen and assign succeeds", False, error)
                has("reopen and assign record", "T1 todo", body)
                _, feed = closed.call("board_feed", {"after": "0", "kind": "updated"})
                has("reopen event names the recipient", "coder", feed)
        for status in ["done", "canceled"]:
            with Server(os.path.join(scratch, "reopen-" + status), "manager") as pair:
                pair.start()
                pair.call("task_add", {"title": "parent"})
                pair.call("task_add", {"title": "child", "parent": 1, "status": "done"})
                pair.call("task_update", {"id": 1, "if_revision": 1, "status": status})
                _, before = pair.call("task_list", {"all": True})
                _, feed_before = pair.call("board_feed", {"after": "0"})
                reply = pair.send("tools/call", {"name": "task_update", "arguments": {"id": 2, "if_revision": 1, "assignee": "coder", "new_actor": True}})
                check("closed pair assign-only message", "done task cannot change assignee; reopen it with an open status in the same update", reply.get("error", {}).get("message"))
                for force in [False, True]:
                    args = {"id": 2, "if_revision": 1, "status": "todo", "assignee": "coder", "new_actor": True, "comment": "must not land"}
                    if force:
                        args.update({"force": True, "reason": "override"})
                    reply = pair.send("tools/call", {"name": "task_update", "arguments": args})
                    check("reopen under closed parent code", -32602, reply.get("error", {}).get("code"))
                    check("reopen under closed parent message", "parent T1 is " + status + "; reopen it before reopening T2", reply.get("error", {}).get("message"))
                    check("reopen under closed parent tasks unchanged", before, pair.call("task_list", {"all": True})[1])
                    check("reopen under closed parent no event", feed_before, pair.call("board_feed", {"after": "0"})[1])
                error, _ = pair.call("task_update", {"id": 1, "if_revision": 2, "status": "todo"})
                check("closed pair parent reopens first", False, error)
                error, body = pair.call("task_update", {"id": 2, "if_revision": 1, "status": "todo", "assignee": "coder", "new_actor": True})
                check("closed pair child then reopens", False, error)
                has("closed pair child record", "T2 todo", body)
                _, feed = pair.call("board_feed", {"after": "0", "kind": "updated"})
                has("closed pair event names the recipient", "coder", feed)

        # task_wait: a row already wanted answers at once, a quiet board answers
        # `timeout:` as ordinary text, and refusals stay JSON-RPC errors.
        with Server(os.path.join(scratch, "wait"), "manager") as waits:
            waits.start()
            waits.call("task_add", {"title": "already done", "status": "done"})
            is_error, body = waits.call("task_wait", {"ids": [1]})
            check("task_wait is not an error", False, is_error)
            check("task_wait answers matched", "matched: 1 of 1 watched in done,canceled", body.splitlines()[0])
            has("task_wait prints the watched row", "T1 done", body)
            has("task_wait labels an earlier event as last", "last:", body)
            has("task_wait prints a cursor", "cursor ", body)
            waits.call("task_add", {"title": "stays open"})
            is_error, body = waits.call("task_wait", {"ids": [2], "timeout_s": 1})
            check("a wait timeout is not an error", False, is_error)
            has("a wait timeout starts with timeout:", "timeout:", body.splitlines()[0])
            has("a wait timeout lists the open row", "T2 todo", body)
            has("a wait timeout keeps a cursor", "cursor ", body)
            reply = waits.send("tools/call", {"name": "task_wait", "arguments": {"ids": [1], "status": "queued"}})
            check("wait unknown status code", -32602, reply.get("error", {}).get("code"))
            has("wait unknown status names it", 'unknown task status: "queued"', reply.get("error", {}).get("message", ""))
            reply = waits.send("tools/call", {"name": "task_wait", "arguments": {"ids": [2], "epic": True, "timeout_s": 1}})
            check("wait epic on a leaf code", -32602, reply.get("error", {}).get("code"))
            check("wait epic on a leaf message", "task T2 is not an epic; an epic is a task with children", reply.get("error", {}).get("message"))
            reply = waits.send("tools/call", {"name": "task_wait", "arguments": {"ids": []}})
            check("wait without ids code", -32602, reply.get("error", {}).get("code"))
            check("wait without ids message", "wait needs at least one task id", reply.get("error", {}).get("message"))
            print("task_wait: matched:, timeout:, and -32602 refusals")

        # task_wait assignee: a handoff from another process ends the wait while
        # the status stays put. One Server answers one call at a time, so the
        # handoff is a delayed CLI update on the same board.
        def handoff_during(db, arguments, to="coder"):
            with Server(db, "manager") as waiter:
                waiter.start()
                waiter.call("task_add", {"title": "handoff row", "status": "in_progress", "assignee": "analyst", "new_actor": True})
                tools = waiter.send("tools/list", {})
                schema = next(t for t in tools["result"]["tools"] if t["name"] == "task_wait")
                check("task_wait schema has assignee", True, "assignee" in schema["inputSchema"]["properties"])
                mover = subprocess.Popen(
                    ["sh", "-c", 'sleep 0.5; "$BIN" --db "$DB" update 1 --if-revision 1 --assignee "$TO" --new-actor'],
                    env=dict(os.environ, BIN=BIN, DB=db, TO=to, AGENTBOARD_ACTOR="analyst"),
                    stdout=subprocess.DEVNULL,
                )
                reply = waiter.send("tools/call", {"name": "task_wait", "arguments": dict(arguments, ids=[1], status="review,done,blocked")})
                mover.wait()
                return reply

        def wait_body(reply):
            return "".join(part.get("text", "") for part in reply["result"]["content"])

        reply = handoff_during(os.path.join(scratch, "wait-assignee-any"), {"assignee": True, "timeout_s": 5})
        check("task_wait assignee true is not an error", False, bool(reply["result"].get("isError")))
        check("task_wait assignee true names the handoff", "matched: 0 of 1 watched in blocked,review,done; handoff T1 to coder", wait_body(reply).splitlines()[0])
        has("task_wait assignee true leaves status alone", "T1 in_progress", wait_body(reply))
        reply = handoff_during(os.path.join(scratch, "wait-assignee-off"), {"timeout_s": 1})
        has("task_wait without assignee times out", "timeout:", wait_body(reply).splitlines()[0])
        reply = handoff_during(os.path.join(scratch, "wait-assignee-false"), {"assignee": False, "timeout_s": 1})
        has("task_wait assignee false times out", "timeout:", wait_body(reply).splitlines()[0])
        reply = handoff_during(os.path.join(scratch, "wait-assignee-other"), {"assignee": "reviewer", "timeout_s": 1})
        has("task_wait assignee for another seat times out", "timeout:", wait_body(reply).splitlines()[0])
        reply = handoff_during(os.path.join(scratch, "wait-assignee-named"), {"assignee": "coder", "timeout_s": 5})
        has("task_wait assignee naming the new owner matches", "; handoff T1 to coder", wait_body(reply).splitlines()[0])
        with Server(os.path.join(scratch, "wait-assignee-empty"), "manager") as empty:
            empty.start()
            empty.call("task_add", {"title": "row"})
            reply = empty.send("tools/call", {"name": "task_wait", "arguments": {"ids": [1], "assignee": "", "timeout_s": 1}})
            check("task_wait empty assignee code", -32602, reply.get("error", {}).get("code"))
        print("task_wait assignee: handoff matched, timeout, named, and empty refusal")

        # task_update release: a dead holder's row returns to todo in one write.
        with Server(os.path.join(scratch, "release"), "manager") as releases:
            releases.start()
            releases.call("task_add", {"title": "held by a dead seat"})
            releases.call(
                "task_update",
                {"id": 1, "if_revision": 1, "status": "in_progress", "assignee": "coder", "new_actor": True},
            )
            reply = releases.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 2, "release": True}})
            check("foreign release without force code", -32602, reply.get("error", {}).get("code"))
            has("foreign release names the holder", "held by coder", reply.get("error", {}).get("message", ""))
            is_error, before = releases.call("task_get", {"id": 1})
            reply = releases.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 2, "release": True}})
            _, after = releases.call("task_get", {"id": 1})
            check("a refused release changes nothing", before, after)
            is_error, body = releases.call(
                "task_update",
                {"id": 1, "if_revision": 2, "release": True, "force": True, "reason": "the coder seat hit its quota", "comment": "back in the pool"},
            )
            check("forced release is not an error", False, is_error)
            check("forced release prints ok", "ok", body.splitlines()[0])
            has("forced release returns the todo record", "T1 todo", body)
            has("forced release clears the assignee", "prio  100 -          held by a dead seat", body)
            has("forced release bumps the revision", "revision 3", body)
            has("forced release prints a cursor", "cursor ", body)
            _, feed = releases.call("board_feed", {"kind": "updated", "full": True})
            has("release event carries the comment", "back in the pool", feed)
            reply = releases.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 3, "release": True}})
            check("holder-less release code", -32602, reply.get("error", {}).get("code"))
            check("holder-less release message", "task T1 has no holder to release", reply.get("error", {}).get("message"))
            reply = releases.send(
                "tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 3, "release": True, "status": "todo"}}
            )
            check("release with status code", -32602, reply.get("error", {}).get("code"))
            has("release with status names the choice", "not both", reply.get("error", {}).get("message", ""))
            is_error, before = releases.call("task_get", {"id": 1})
            reply = releases.send(
                "tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 3, "release": True, "priority": 5}}
            )
            check("release with a field flag code", -32602, reply.get("error", {}).get("code"))
            check(
                "release with a field flag message",
                "choose release or a field change, not both",
                reply.get("error", {}).get("message"),
            )
            _, after = releases.call("task_get", {"id": 1})
            check("a release with a field flag changes nothing", before, after)
            print("task_update release: todo record, -32602 held and holder-less refusals")
    finally:
        server.close()

    print(f"{passed}/{passed + failed} passed (mcp, lmdb)")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
