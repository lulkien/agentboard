#!/usr/bin/env python3
"""MCP surface check: twelve tools on one stdio server or daemon, scratch board."""

import http.client
import json
import os
import re
import subprocess
import sys
import tempfile

BIN = os.environ.get("AGENTBOARD_TEST_BIN", os.path.join(os.getcwd(), "target/debug/agentboard"))
HTTP = os.environ.get("AGENTBOARD_MCP_TRANSPORT") == "http"
PROTOCOL = "2025-11-25"
TOOLS = {
    "task_add",
    "task_list",
    "task_get",
    "task_epic",
    "task_update",
    "task_claim",
    "task_comment",
    "board_feed",
    "board_brief",
    "board_agents",
    "board_project_delete",
    "board_agent_delete",
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


class StdioServer:
    def __init__(self, db, actor, args=(), extra_env=None):
        env = dict(os.environ, AGENTBOARD_DB=db, AGENTBOARD_ACTOR=actor)
        if extra_env:
            env.update(extra_env)
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


class HttpServer:
    """Same send/call interface over `agentboard daemon`: one daemon per instance.

    The seat name is the URL segment; a daemon never reads its own environment, so
    an exported AGENTBOARD_PROJECT becomes `?project=` on the URL instead.
    """

    def __init__(self, db, actor, args=(), extra_env=None):
        args = list(args)
        if "--actor" in args:
            actor = args[args.index("--actor") + 1]
        project = (extra_env or {}).get("AGENTBOARD_PROJECT")
        self.path = f"/mcp/{actor}" + (f"?project={project}" if project else "")
        env = {k: v for k, v in os.environ.items() if not k.startswith("AGENTBOARD_")}
        if extra_env and "AGENTBOARD_MAP_SIZE_MB" in extra_env:
            env["AGENTBOARD_MAP_SIZE_MB"] = extra_env["AGENTBOARD_MAP_SIZE_MB"]
        self.proc = subprocess.Popen(
            [BIN, "--db", db, "daemon", "127.0.0.1:0"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            text=True,
        )
        line = self.proc.stdout.readline()
        if "listening on http://" not in line:
            raise SystemExit(f"daemon did not start: {line!r} {self.proc.stderr.read()}")
        host, port = line.split("http://")[1].split()[0].rsplit(":", 1)
        self.conn = http.client.HTTPConnection(host, int(port), timeout=30)
        self.seq = 0

    def send(self, method, params=None, notification=False):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if not notification:
            self.seq += 1
            message["id"] = self.seq
        self.conn.request(
            "POST",
            self.path,
            json.dumps(message),
            {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"},
        )
        response = self.conn.getresponse()
        body = response.read().decode()
        if notification:
            return None
        if response.status != 200:
            raise SystemExit(f"daemon answered {response.status}: {body}")
        return json.loads(body)

    call = StdioServer.call
    start = StdioServer.start

    def close(self):
        self.conn.close()
        self.proc.kill()
        self.proc.wait()
        self.proc.stdout.close()
        self.proc.stderr.close()

    __enter__ = StdioServer.__enter__
    __exit__ = StdioServer.__exit__


Server = HttpServer if HTTP else StdioServer


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
        check("tools/list is the twelve-tool surface", TOOLS, names)
        check("tools/list has no task_wait", False, "task_wait" in names)
        tools_by_name = {tool["name"]: tool for tool in listed["result"]["tools"]}
        read_tools = {"task_list", "task_get", "task_epic", "board_feed", "board_brief", "board_agents"}
        delete_tools = {"board_project_delete", "board_agent_delete"}
        check("read-only tool annotations", read_tools, {name for name, tool in tools_by_name.items() if tool.get("annotations", {}).get("readOnlyHint") is True})
        check("destructive tool annotations", delete_tools, {name for name, tool in tools_by_name.items() if tool.get("annotations", {}).get("destructiveHint") is True})
        check("priority argument description", True, "lower" in tools_by_name["task_add"]["inputSchema"]["properties"]["priority"]["description"].lower())
        check("after argument description", True, "instance" in tools_by_name["board_feed"]["inputSchema"]["properties"]["after"]["description"] and "0" in tools_by_name["board_feed"]["inputSchema"]["properties"]["after"]["description"])
        check("kind argument values", True, all(value in tools_by_name["board_feed"]["inputSchema"]["properties"]["kind"]["description"] for value in ("created", "updated", "claimed", "commented")))
        reply = server.send("tools/call", {"name": "task_wait", "arguments": {"ids": [1]}})
        check("task_wait call is refused", True, "error" in reply or bool(reply["result"].get("isError")))

        is_error, body = server.call("task_add", {"title": "mcp task", "description": "full text here"})
        check("task_add is not an error", False, is_error)
        has("task_add returns the record", "T1 todo", body)
        cursor_match = re.search(r"cursor ([0-9a-f]+:[0-9]+)", body)
        check("task_add returns an instance cursor", True, cursor_match is not None)
        is_error, body = server.call("task_add", {"title": "unknown assignee", "assignee": "nobody"})
        check("unknown MCP assignee is refused", True, is_error)
        has("unknown assignee advice matches CLI", 'use new_actor / --new-actor to register the target', body)
        is_error, body = server.call("task_add", {"title": "oversize", "description": "d" * 32769})
        check("oversized description uses invalid params", True, is_error)
        has("oversized description names size and limit", "description is 32769 characters; the limit is 32768", body)
        is_error, body = server.call("task_comment", {"id": 1, "body": "c" * 32769})
        check("oversized comment uses invalid params", True, is_error)
        has("oversized comment names size and limit", "comment is 32769 characters; the limit is 32768", body)

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
        has("task_claim returns a work order", "implementing", body)
        is_error, body = server.call("task_claim")
        has("an empty claim answers no work", "no ready tasks", body)

        is_error, body = server.call("task_comment", {"id": 1, "body": "notes from mcp"})
        has("task_comment acknowledges", "ok", body)
        check("task_comment returns an instance cursor", True, re.search(r"cursor [0-9a-f]+:[0-9]+", body) is not None)

        is_error, body = server.call("board_feed", {"kind": "commented"})
        has("board_feed finds the comment", "notes from mcp", body)
        has("board_feed prints a cursor", "cursor ", body)

        is_error, body = server.call("board_brief", {})
        has("board_brief counts implementing", "implementing 1", body)
        check("board_brief reports map usage", True, re.search(r"^map used [0-9]+\.[0-9]% \([0-9]+ of [0-9]+ bytes\)$", body, re.M) is not None)

        is_error, body = server.call("board_agents", {})
        has("board_agents lists the actor", "worker", body)

        is_error, body = server.call("task_update", {"id": 1, "if_revision": 1, "title": "stale"})
        check("stale revision is an error", True, is_error)
        has("stale revision names the numbers", "stale task revision 1", body)

        is_error, body = server.call("task_update", {"id": 1, "if_revision": 2, "title": "updated"})
        check("task_update returns an instance cursor", False, is_error)
        check("task_update cursor has instance shape", True, re.search(r"cursor [0-9a-f]+:[0-9]+", body) is not None)

        is_error, body = server.call("task_update", {"id": 1, "if_revision": 3, "force": True, "title": "x"})
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

        # Reports append by default, replace on request, and only full reads show them.
        _, body = server.call(
            "task_update",
            {"id": 2, "if_revision": 1, "coder_report": "## Coder report 01\nbuilt"},
        )
        has("report update moves the revision", "revision 2", body)
        _, body = server.call("task_get", {"id": 2})
        check("compact task_get hides reports", False, "## Coder report 01" in body)
        _, body = server.call("task_get", {"id": 2, "full": True})
        has("full task_get shows the report", "## Coder report 01", body)
        _, body = server.call(
            "task_update",
            {"id": 2, "if_revision": 2, "coder_report": "## Coder report 02\nfixed"},
        )
        _, body = server.call("task_get", {"id": 2, "full": True})
        has("a second report keeps the first", "## Coder report 01", body)
        has("a second report adds a section marker", "\n\n---\n\n", body)
        _, body = server.call(
            "task_update",
            {
                "id": 2,
                "if_revision": 3,
                "coder_report": "short",
                "replace_report": True,
            },
        )
        has("replace report moves the revision", "revision 4", body)
        _, body = server.call("task_get", {"id": 2, "full": True})
        check("replace drops the old sections", False, "## Coder report 01" in body)
        has("replace stores the new body", "coder_report short", body)
        is_error, body = server.call(
            "task_update", {"id": 2, "if_revision": 4, "analyst_report": "  "}
        )
        check("empty report append is an error", True, is_error)
        has("empty report append names the rule", "report body must be nonempty", body)

        # A comment with no field change is a comment; claiming your own todo starts it.
        is_error, body = server.call("task_add", {"title": "own todo", "assignee": "worker", "new_actor": True})
        has("assigned todo created", "ok", body)
        is_error, body = server.call("task_claim", {"id": 3})
        has("claiming your assigned todo starts it", "implementing", body)
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
        project_db = os.path.join(scratch, "project-scope")
        for name, path in [("A", "/project-a"), ("B", "/project-b")]:
            subprocess.run(
                [BIN, "--db", project_db, "project", "add", "--name", name, "--path", path],
                check=True,
                capture_output=True,
                text=True,
            )
        with Server(
            project_db, "worker", extra_env={"AGENTBOARD_PROJECT": "2"}
        ) as scoped:
            scoped.start()
            scoped.call("task_add", {"title": "A scoped row", "status": "reviewing", "project": 1})
            scoped.call("task_add", {"title": "B scoped row", "status": "reviewing", "project": 2})
            _, body = scoped.call("task_list", {"project": 1})
            has("task_list explicit project beats environment", "A scoped row", body)
            check("task_list explicit project excludes environment row", False, "B scoped row" in body)
            _, body = scoped.call("task_list")
            has("task_list defaults to AGENTBOARD_PROJECT", "B scoped row", body)
            check("task_list environment scope excludes other project", False, "A scoped row" in body)
            _, body = scoped.call("board_brief", {"project": 1})
            has("board_brief accepts explicit project", "A scoped row", body)
            check("board_brief explicit project excludes other project", False, "B scoped row" in body)
            _, body = scoped.call("board_brief")
            has("board_brief defaults to AGENTBOARD_PROJECT", "B scoped row", body)
            check("board_brief environment scope excludes other project", False, "A scoped row" in body)
        delete_db = os.path.join(scratch, "delete")
        for name in ("A", "B"):
            subprocess.run([BIN, "--db", delete_db, "project", "add", "--name", name, "--path", "/" + name], check=True, capture_output=True, text=True)
        with Server(delete_db, "worker") as deleting:
            deleting.start()
            deleting.call("task_add", {"title": "linked", "project": 1})
            reply = deleting.send("tools/call", {"name": "board_project_delete", "arguments": {"id": 1}})
            check("project delete in use is JSON-RPC error", -32602, reply.get("error", {}).get("code"))
            has("project delete in use count", "project 1 still has tasks (count 1); move or detach them first", json.dumps(reply))
            reply = deleting.send("tools/call", {"name": "board_project_delete", "arguments": {"id": 0}})
            check("built-in project delete is JSON-RPC error", -32602, reply.get("error", {}).get("code"))
            error, body = deleting.call("board_project_delete", {"id": 2})
            check("project delete success", (False, "deleted project 2"), (error, body))
            reply = deleting.send("tools/call", {"name": "board_project_delete", "arguments": {"id": 2}})
            has("project delete missing", "project 2 does not exist", json.dumps(reply))
            deleting.call("task_update", {"id": 1, "if_revision": 1, "clear_project": True})
            error, body = deleting.call("board_project_delete", {"id": 1})
            check("project delete after detach", (False, "deleted project 1"), (error, body))
            deleting.call("task_add", {"title": "open", "assignee": "bob", "new_actor": True})
            deleting.call("task_add", {"title": "closed", "assignee": "bob", "status": "done"})
            reply = deleting.send("tools/call", {"name": "board_agent_delete", "arguments": {"name": "bob"}})
            check("agent delete open holder is JSON-RPC error", -32602, reply.get("error", {}).get("code"))
            has("agent delete open count", 'agent \\"bob\\" still holds open tasks (count 1); release or reassign them first', json.dumps(reply))
            reply = deleting.send("tools/call", {"name": "board_agent_delete", "arguments": {"name": "ghost"}})
            has("agent delete missing", 'agent \\"ghost\\" does not exist', json.dumps(reply))
            deleting.call("task_update", {"id": 2, "if_revision": 1, "clear_assignee": True})
            error, body = deleting.call("board_agent_delete", {"name": "bob"})
            check("agent delete success", (False, "deleted agent bob"), (error, body))
            _, body = deleting.call("board_agents")
            check("agent delete removes roster entry", False, "bob" in body)
            _, body = deleting.call("task_get", {"id": 3})
            has("agent delete keeps closed holder", "bob", body)
            reply = deleting.send("tools/call", {"name": "board_agent_delete", "arguments": {}})
            check("agent delete missing argument", True, bool(reply.get("result", {}).get("isError")))
            reply = deleting.send("tools/call", {"name": "board_project_delete", "arguments": {}})
            check("project delete missing argument", True, bool(reply.get("result", {}).get("isError")))
        # CLI and MCP share one snapshot and renderer, with no intervening writes.
        with Server(os.path.join(scratch, "epics"), "worker") as epics:
            epics.start()
            _, body = epics.call("task_add", {"title": "umbrella", "priority": 20})
            for status in ["todo", "done", "canceled"]:
                epics.call("task_add", {"title": status + " child", "parent": 1, "status": status})
            epics.call("task_add", {"title": "ordinary", "status": "reviewing", "priority": -1})
            error, body = epics.call("task_epic", {"id": 1})
            check("task_epic succeeds", False, error)
            check("epic four lines", 4, len(body.splitlines()))
            check("epic direct counts", "todo 1  planning 0  implementing 0  blocked 0  reviewing 0  done 1  canceled 1", body.splitlines()[1])
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
            check("brief excludes children", 4, len(brief.splitlines()))
            check("brief map usage before counts", True, brief.splitlines()[-2].startswith("map used "))
            check("brief global counts last", "todo 2  planning 0  implementing 0  blocked 0  reviewing 1  done 1  canceled 1", brief.splitlines()[-1])
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

        # task_update release: a dead holder's row returns to todo in one write.
        with Server(os.path.join(scratch, "release"), "manager") as releases:
            releases.start()
            releases.call("task_add", {"title": "held by a dead seat"})
            releases.call(
                "task_update",
                {"id": 1, "if_revision": 1, "status": "implementing", "assignee": "coder", "new_actor": True},
            )
            reply = releases.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 2, "release": True}})
            check("foreign release without force code", -32602, reply.get("error", {}).get("code"))
            has("foreign release names the holder", "held by coder", reply.get("error", {}).get("message", ""))
            check(
                "held refusal uses adapter-neutral advice",
                "task is held by coder; force with a reason takes it, or release with force and a reason returns it to todo",
                reply.get("error", {}).get("message", ""),
            )
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
            # planning is held work; legacy status inputs are accepted and never printed.
            is_error, body = releases.call("task_add", {"title": "planned", "status": "planning", "assignee": "analyst", "new_actor": True})
            check("planning with a holder is not an error", False, is_error)
            has("planning prints its own name", "planning", body)
            is_error, body = releases.call("task_update", {"id": 2, "if_revision": 1, "status": "review", "force": True, "reason": "test legacy input"})
            has("legacy review input prints reviewing", "reviewing", body)
            print("task_update release: todo record, -32602 held and holder-less refusals")

        fault_env = {"AGENTBOARD_MAP_SIZE_MB": "1"}
        with Server(os.path.join(scratch, "typed-errors"), "manager", extra_env=fault_env) as faults:
            faults.start()
            faults.call("task_add", {"title": "survives store fault"})
            reply = faults.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 1, "coder_report": "r" * (262 * 1024 + 1)}})
            check("oversized report uses invalid params", -32602, reply.get("error", {}).get("code"))
            has("oversized report names its limit", "coder_report is 268289 characters; the limit is 262144", reply.get("error", {}).get("message", ""))
            reply = faults.send("tools/call", {"name": "task_update", "arguments": {"id": 1, "if_revision": 0, "title": "stale"}})
            check("store refusal uses invalid params", -32602, reply.get("error", {}).get("code"))
            error = None
            for index in range(16):
                reply = faults.send("tools/call", {"name": "task_add", "arguments": {"title": f"fills map {index}", "description": "界" * 32768, "goal": "界" * 32768}})
                if "error" in reply:
                    error = reply["error"]
                    break
            check("store fault uses internal error", -32603, (error or {}).get("code"))
            has("store fault preserves map-size guidance", "AGENTBOARD_MAP_SIZE_MB", (error or {}).get("message", ""))
            is_error, body = faults.call("task_get", {"id": 1})
            check("server survives a store fault", False, is_error)
            has("surviving row remains readable", "survives store fault", body)
            print("typed store errors: refusal -32602, map full -32603")
    finally:
        server.close()

    print(f"{passed}/{passed + failed} passed ({'mcp-http' if HTTP else 'mcp'}, lmdb)")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
