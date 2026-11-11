#!/usr/bin/env python3
"""Daemon check: `agentboard daemon` on scratch boards and loopback ports.

Every daemon binds 127.0.0.1:0 and reports its port on the startup line; nothing
here touches a live board or a fixed port.
"""

import http.client
import json
import os
import re
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


def ok(name, condition):
    global passed, failed
    if condition:
        passed += 1
    else:
        failed += 1
        print(f"FAIL: {name}")


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

    def fetch(self, method, path, headers=None):
        conn = http.client.HTTPConnection(self.host, self.port, timeout=30)
        conn.request(method, path, None, headers or {})
        response = conn.getresponse()
        data = response.read()
        head = {name.lower(): value for name, value in response.getheaders()}
        conn.close()
        return response.status, head, data

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


def get_json(daemon, target):
    status, body = daemon.request("GET", target)
    return status, (json.loads(body) if status == 200 else body)


def feed_cursor(db):
    match = re.search(r"^cursor (.+)$", cli(db, "feed", "--after", "0", "--limit", "500").stdout, re.MULTILINE)
    return match.group(1)


def api_checks(scratch):
    """The read-only JSON API, on the fixture the HTML dashboard suite used."""
    db = os.path.join(scratch, "api")

    def add(*args):
        out = cli(db, "--actor", "manager", "add", *args)
        assert out.returncode == 0, out.stderr
        return out.stdout.split()[1].removeprefix("T")

    cli(db, "--actor", "manager", "project", "add", "--name", "dashboard", "--path", "/dashboard")
    unbroken = "T90" + "x" * 195 + "<&"
    spaced = "T90 long title " + "words " * 30 + "words"
    add(unbroken)
    add("Done child", "--parent", "1", "--status", "done")
    description = 'A "quoted" <tag>& first line\nsecond line'
    goal = "Goal <done>&\nkeep this line"
    add("<script>alert('x')</script>", "--parent", "1", "--assignee", "alice<&", "--new-actor",
        "--project", "1", "--description", description, "--goal", goal)
    cli(db, "--actor", "manager", "update", "3", "--if-revision", "1", "--priority", "-7")
    report = "Coder <b>&\nsecond line"
    cli(db, "--actor", "manager", "update", "3", "--if-revision", "2", "--coder-report", report)
    comment = 'Comment "quoted" <b>&\nsecond line'
    cli(db, "--actor", "manager", "comment", "3", "--body", comment)
    add("Planning child", "--parent", "1", "--status", "planning", "--assignee", "manager")
    add("Implementing child", "--parent", "1", "--status", "implementing", "--assignee", "manager")
    add("Blocked child", "--parent", "1", "--status", "blocked", "--assignee", "manager")
    add("Reviewing child", "--parent", "1", "--status", "reviewing")
    for index in range(43):
        add(f"Page task {index:02}")
    cli(db, "--actor", "manager", "update", "8", "--if-revision", "1", "--title", spaced)
    add(unbroken)
    add(spaced)
    cli(db, "--actor", "manager", "project", "add", "--name", '<b>p&"</b>', "--path", "/other")
    add("Other project row", "--project", "2")
    epic = add("Escaped epic <script>")
    add("Escaped epic child", "--parent", epic, "--status", "done")
    cli(db, "--actor", "manager", "update", epic, "--if-revision", "1", "--status", "canceled")
    add("Canceled fixture row", "--status", "canceled")

    daemon = Daemon(db)
    try:
        cursor = feed_cursor(db)
        port = daemon.port

        # Board: lane cap with totals, attention order, held counts.
        status, board = get_json(daemon, "/api/board")
        check("board is 200", 200, status)
        lanes = {lane["status"]: lane for lane in board["lanes"]}
        check("lanes are the five open statuses in order", ["todo", "planning", "implementing", "blocked", "reviewing"], [lane["status"] for lane in board["lanes"]])
        check("todo lane is capped at 8", 8, len(lanes["todo"]["tasks"]))
        check("todo lane total is 48", 48, lanes["todo"]["total"])
        check("status names are client-facing", "implementing", lanes["implementing"]["tasks"][0]["status"])
        check("attention order is blocked, assigned todo, reviewing", [6, 3, 7], [t["id"] for t in board["attention"]["tasks"]])
        check("attention total", 3, board["attention"]["total"])
        held = {h["assignee"]: h["total"] for h in board["held"]}
        check("held counts", {"manager": 3, "alice<&": 1}, held)
        counts = {c["status"]: c["count"] for c in board["counts"]}
        check("board counts: done 2, canceled 2", (2, 2), (counts["done"], counts["canceled"]))
        check("board summaries carry no report text", False, "coder_report" in json.dumps(board))
        check("board agents are listed", True, {"manager", "alice<&"} <= {a["name"] for a in board["agents"]})
        check("board cursor is instance:seq", cursor, board["cursor"])
        status, two = get_json(daemon, "/api/board?project=2")
        check("project 2 board holds its one todo", [53], [t["id"] for t in {l["status"]: l for l in two["lanes"]}["todo"]["tasks"]])
        check("project 2 has no held rows", [], two["held"])
        check("project 2 still lists the roster", True, "manager" in {a["name"] for a in two["agents"]})
        status, one = get_json(daemon, "/api/board?project=1")
        check("project 1 board holds only its row", [3], [t["id"] for l in one["lanes"] for t in l["tasks"]])

        # Tasks: newest first, paging, status counts, filters.
        status, page = get_json(daemon, "/api/tasks")
        check("tasks page one is newest first 56..37", list(range(56, 36, -1)), [t["id"] for t in page["tasks"]])
        check("tasks total and more", (56, True), (page["total"], page["more"]))
        all_counts = {c["status"]: c["count"] for c in page["counts"]}
        check("status counts: All 56 / todo 48", (56, 48), (sum(all_counts.values()), all_counts["todo"]))
        status, page2 = get_json(daemon, "/api/tasks?offset=20")
        check("offset 20 continues 36..17", list(range(36, 16, -1)), [t["id"] for t in page2["tasks"]])
        status, last = get_json(daemon, "/api/tasks?offset=40")
        check("last page has 16 rows and no more", (16, False), (len(last["tasks"]), last["more"]))
        status, todo = get_json(daemon, "/api/tasks?status=todo")
        check("status filter total", 48, todo["total"])
        status, mine = get_json(daemon, "/api/tasks?assignee=alice%3C%26")
        check("assignee filter is percent-decoded", [3], [t["id"] for t in mine["tasks"]])
        status, kids = get_json(daemon, "/api/tasks?parent=1")
        check("parent filter lists the children", [2, 3, 4, 5, 6, 7], sorted(t["id"] for t in kids["tasks"]))
        status, scoped = get_json(daemon, "/api/tasks?project=2")
        check("project 2 lists only its row", [53], [t["id"] for t in scoped["tasks"]])
        check("scoped status counts add up", 1, sum(c["count"] for c in scoped["counts"]))

        # Detail.
        status, detail = get_json(daemon, "/api/tasks/3")
        check("detail is 200", 200, status)
        task = detail["task"]
        check("detail keeps description bytes", description, task["description"])
        check("detail keeps goal bytes", goal, task["goal"])
        check("detail keeps report bytes", report, task["coder_report"])
        check("detail priority", -7, task["priority"])
        check("detail has no epic for a leaf", None, detail["epic"])
        kinds = [event["kind"] for event in detail["events"]]
        check("detail events are this task's, oldest first", ["created", "updated", "updated", "commented"], kinds)
        check("detail comment text", comment, detail["events"][-1]["payload"]["text"])
        status, parent = get_json(daemon, "/api/tasks/1")
        check("a parent row carries its rollup", 6, parent["epic"]["total"])
        check("a parent row lists its children", 6, parent["children"]["total"])

        # Epics.
        status, epics = get_json(daemon, "/api/epics")
        check("epics are 200", 200, status)
        by_id = {e["task"]["id"]: e for e in epics["epics"]}
        check("epic ids", [1, 54], [e["task"]["id"] for e in epics["epics"]])
        check("epic children are capped at 4 with the total", (4, 6), (len(by_id[1]["children"]["tasks"]), by_id[1]["children"]["total"]))
        check("epic rollup", (1, 6), (by_id[1]["done"], by_id[1]["total"]))
        check("epic depth is 0 for a root", 0, by_id[1]["depth"])
        check("epic 1 child ids", [3, 2, 4, 5], [t["id"] for t in by_id[1]["children"]["tasks"]])
        check("epic row keys", {"task", "depth", "counts", "done", "total", "children"}, set(by_id[1]))
        check("epic children keys", {"total", "tasks"}, set(by_id[1]["children"]))
        check("epic 1 counts", {"todo": 1, "planning": 1, "implementing": 1, "blocked": 1, "reviewing": 1, "done": 1, "canceled": 0}, {x["status"]: x["count"] for x in by_id[1]["counts"]})
        check("epic 54 children", ([55], 1, 1), ([t["id"] for t in by_id[54]["children"]["tasks"]], by_id[54]["children"]["total"], by_id[54]["done"]))
        check("project 1 epics empty", {"project": 1, "epics": []}, get_json(daemon, "/api/epics?project=1")[1])
        check("project 2 epics empty", {"project": 2, "epics": []}, get_json(daemon, "/api/epics?project=2")[1])

        # Feed: newest page, before paging, filters, project isolation.
        status, newest = get_json(daemon, "/api/feed")
        check("feed page is 20 events", 20, len(newest["events"]))
        check("feed says older exists", True, newest["older"])
        seqs = [e["seq"] for e in newest["events"]]
        check("feed is in ascending order", sorted(seqs), seqs)
        status, older = get_json(daemon, "/api/feed?before=" + urllib.parse.quote(newest["before"], safe=""))
        older_seqs = [e["seq"] for e in older["events"]]
        check("older page ends before the newest page", True, max(older_seqs) < min(seqs))
        check("older page is full", 20, len(older_seqs))
        check("feed event keeps the stored actor", "manager", newest["events"][0]["actor"])
        status, filtered = get_json(daemon, "/api/feed?task=3&kind=commented")
        check("feed filter", [3], sorted({e["task_id"] for e in filtered["events"]}))
        check("feed kind filter", ["commented"], sorted({e["kind"] for e in filtered["events"]}))
        check("filtered feed is exhausted", (False, None), (filtered["older"], filtered["before"]))
        status, isolated = get_json(daemon, "/api/feed?project=2")
        check("project feed holds only that project's rows", {53}, {e["task_id"] for e in isolated["events"]})
        status, listing = get_json(daemon, "/api/projects")
        check("projects are listed", ["dashboard", '<b>p&"</b>'], [p["project_name"] for p in listing["projects"]])

        # Refusals.
        for target in ["/api/tasks?status=bogus", "/api/tasks?foo=1", "/api/tasks?status=todo&status=done",
                       "/api/tasks?offset=x", "/api/tasks?parent=0", "/api/board?offset=1", "/api/board?status=todo",
                       "/api/epics?task=1", "/api/feed?before=zzz", "/api/feed?before=foreign:5", "/api/feed?kind=bogus",
                       "/api/feed?status=todo", "/api/projects?kind=x", "/api/tasks/abc", "/api/tasks/0",
                       "/api/board?project=0", "/api/board?project=x", "/api/board?project=1&project=1",
                       "/api/tasks?x", "/api/tasks?&", "/api/tasks?status=%zz"]:
            check(f"400 for {target}", 400, daemon.request("GET", target)[0])
        for target in ["/api/tasks/999", "/api/tasks?parent=999", "/api/board?project=9", "/api/feed?project=9",
                       "/api/epics?project=9", "/api/projects?project=9", "/api/tasks/1/extra", "/api/nope"]:
            check(f"404 for {target}", 404, daemon.request("GET", target)[0])
        for target in ["/api/board", "/api/epics", "/api/tasks", "/api/tasks/3", "/api/feed", "/api/projects"]:
            for method in ["POST", "PUT", "PATCH", "DELETE", "HEAD"]:
                check(f"{method} {target} is 405", 405, daemon.request(method, target, "{}")[0])
        check("a foreign Host cannot read the API", 421, daemon.request("GET", "/api/board", headers={"Host": "evil.example"})[0])
        check("a foreign Origin cannot read the API", 403, daemon.request("GET", "/api/tasks", headers={"Origin": "http://evil.example"})[0])
        check("our Origin reads the API", 200, daemon.request("GET", "/api/tasks", headers={"Origin": f"http://127.0.0.1:{port}"})[0])
        check("no GET moved the feed cursor", cursor, feed_cursor(db))
    finally:
        daemon.close()


CSP = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def css_checks(css):
    """The stylesheet rules the HTML dashboard suite pinned, on the embedded file."""
    ok("blocked column heading retains its hatch", re.search(r"\.column-head\.status-blocked\s*\{[^}]*background-image:none", css))
    ok("empty card slot lacks its dashed shape", re.search(r"\.card-list > \.empty\s*\{[^}]*border:1px dashed", css))
    ok("card does not own its background rule", re.search(r"(?m)^\.card\s*\{[^}]*background:var\(--strip\)", css))
    ok("shared panel/card padding rule remains", not re.search(r"\.panel\s*,\s*\.card", css))
    ok("more row is not strip shaped", re.search(r"\.column-more\s*\{[^}]*min-height:36px", css))
    ok("page head does not wrap its title and aside", re.search(r"\.page-head\s*\{[^}]*display:flex[^}]*flex-wrap:wrap", css))
    ok("status chips are shorter than 36px", re.search(r"\.status-chip\s*\{[^}]*min-height:36px", css))
    ok("phone topbar remains sticky", re.search(r"@media\s*\(max-width:650px\)\s*\{[^}]*\.topbar\s*\{[^}]*position:static", css))
    phone_rules = css.split("@media (max-width:650px)", 1)[1].split("@media (prefers-reduced-motion", 1)[0]
    ok("direct timeline events are missing their marker rule", re.search(r"\.timeline\s*>\s*li::before\s*\{[^}]*background:var\(--implementing\)", css))
    ok("seat names can overflow their chips", re.search(r"\.seat-name\s*\{[^}]*text-overflow:ellipsis[^}]*min-width:0", css))
    ok("kanban does not use five shrinkable columns", re.search(r"\.kanban\s*\{[^}]*grid-template-columns:repeat\(5,minmax\(0,1fr\)\)", css))
    ok("card titles lack the three-line clamp and long-token fallback", re.search(r"\.card-title\s*\{[^}]*-webkit-line-clamp:3[^}]*overflow:hidden[^}]*overflow-wrap:anywhere[^}]*max-height:", css))
    ok("cards keep an intrinsic minimum width", re.search(r"(?m)^\.card\s*\{[^}]*min-width:0", css))
    ok("column headings are not sticky", re.search(r"\.column-head\s*\{[^}]*position:sticky", css))
    ok("kanban does not stack on narrow screens", re.search(r"@media\s*\(max-width:899px\)\s*\{[^}]*\.kanban\s*\{[^}]*grid-template-columns:1fr", css))
    ok("attention rail is not a two-column strip grid", re.search(r"\.attention \.strip-list\s*\{[^}]*grid-template-columns:repeat\(2,minmax\(0,1fr\)\)", css))
    ok("attention rail does not collapse to one column on narrow screens", re.search(r"@media\s*\(max-width:899px\)\s*\{[^}]*\.attention \.strip-list\s*\{[^}]*grid-template-columns:1fr", css))
    ok("top bar is not sticky, layered and opaque", re.search(r"\.topbar\s*\{[^}]*position:sticky[^}]*top:0[^}]*z-index:\d+[^}]*background:var\(--strip\)", css))
    ok("anchors do not account for the sticky header", "scroll-padding-top:70px" in css and "scroll-margin-top:70px" in css)
    ok("header control sizing is not shared", re.search(r"\.header-control\s*\{[^}]*min-height:40px[^}]*padding:\.5rem \.75rem[^}]*font:inherit", css))
    ok("header does not wrap at narrow widths", ".topbar {" in css and "flex-wrap:wrap" in css)
    ok("Show button does not share the theme control padding", ".project-switcher button[type=submit] { padding:.5rem .75rem; }" in css)
    ok("page layout does not keep the footer at the viewport bottom", "body { min-height:100vh; display:flex; flex-direction:column; }" in css and "main { flex:1;" in css)
    ok("footer is not a slim 40px bar", re.search(r"(?m)^footer\s*\{[^}]*min-height:40px[^}]*padding:0 max\(1rem", css))
    ok("footer controls are not compact and centered", "footer .header-control { min-height:28px; padding:.25rem .6rem; font:600 13px/1.2 var(--signage); }" in css and re.search(r"(?m)^footer\s*\{[^}]*justify-content:center", css))
    ok("footer must not be fixed or sticky", not re.search(r"(?m)^footer[^{}]*\{[^}]*position:", css))
    ok("main bottom gap above the footer was not reduced", "margin:1.5rem auto;" in css and "main { margin:1rem auto; }" in phone_rules)
    ok("system theme defaults are missing", "prefers-color-scheme" in css)
    ok("feed change details do not inherit the dashboard font", "pre { font:inherit; }" in css)
    ok("reduced motion or keyboard focus style is missing", "prefers-reduced-motion" in css and ":focus-visible" in css)
    ok("tasks table must preserve intrinsic cell widths and scroll inside its wrapper without per-letter wrapping", re.search('\\.table-scroll\\s*\\{[^}]*overflow-x:auto', css) and re.search('\\.task-table\\s*\\{[^}]*width:max-content[^}]*min-width:100%', css) and (not re.search('(?:\\.path\\s*,\\s*td|td)\\s*\\{[^}]*overflow-wrap:anywhere', css)))
    ok("task titles are not width-capped, clamped to three lines, and protected from unbroken tokens", re.search(r"\.task-table \.task-title\s*\{[^}]*min-width:11rem[^}]*max-width:11rem", css) and re.search(r"\.task-table \.task-title > span\s*\{[^}]*-webkit-line-clamp:3[^}]*overflow:hidden[^}]*overflow-wrap:anywhere", css))
    ok("non-title task cells are not kept to one line", re.search(r"\.task-table td:not\(\.task-title\)\s*\{(?=[^}]*white-space:nowrap)(?=[^}]*text-overflow:ellipsis)(?=[^}]*overflow:hidden)[^}]*\}", css))
    ok("task table scroll cue edge and shadow are missing", re.search(r"\.table-scroll\s*\{[^}]*border-right:2px solid var\(--muted\)[^}]*box-shadow:inset -8px 0 8px -8px var\(--ink\)", css))
    ok("strip list items keep their intrinsic min-content width", re.search(r"\.strip-list\s*>\s*li\s*\{[^}]*min-width:0", css))
    ok("strip titles do not permit long words to shrink", re.search(r"\.strip-title\s*\{[^}]*overflow-wrap:anywhere", css))
    ok("unbroken detail titles can overflow their page", re.search(r"\.issue-header h2\s*\{[^}]*overflow-wrap:anywhere", css))
    ok("no redundant card flag rule", "card-flag" not in css)


def static_checks(daemon):
    """The embedded dashboard: one shell for every page path, no inline code."""
    shell = open(os.path.join(ROOT, "dashboard", "index.html"), "rb").read()
    browser_headers = {
        "content-security-policy": CSP,
        "cache-control": "no-store",
        "x-content-type-options": "nosniff",
        "referrer-policy": "no-referrer",
    }
    for path in ["/", "/epics", "/tasks", "/tasks/1", "/tasks/999", "/feed", "/tasks?project=1&status=todo",
                 "/feed?task=1&kind=commented", "/epics?project=2"]:
        status, head, body = daemon.fetch("GET", path)
        check(f"{path} is 200", 200, status)
        check(f"{path} serves the embedded shell", shell, body)
        check(f"{path} is html", "text/html; charset=utf-8", head.get("content-type"))
        for name, value in browser_headers.items():
            check(f"{path} sends {name}", value, head.get(name))
    check("HEAD / is allowed", 200, daemon.fetch("HEAD", "/")[0])
    for method in ["POST", "PUT", "PATCH", "DELETE"]:
        check(f"{method} / is 405", 405, daemon.fetch(method, "/")[0])
        check(f"{method} /tasks/1 is 405", 405, daemon.fetch(method, "/tasks/1")[0])
    for path in ["/tasks/1/extra", "/elsewhere", "/assets/nope", "/assets/index.html", "/index.html", "/app.js"]:
        check(f"{path} is 404", 404, daemon.fetch("GET", path)[0])
    status, head, _ = daemon.fetch("GET", "/", {"Host": "evil.example"})
    check("a foreign Host cannot load the shell", 421, status)
    check("a refusal still carries nosniff", "nosniff", head.get("x-content-type-options"))
    status, head, _ = daemon.fetch("GET", "/api/board")
    check("API responses are no-store", "no-store", head.get("cache-control"))
    check("API responses are nosniff", "nosniff", head.get("x-content-type-options"))

    status, head, script = daemon.fetch("GET", "/assets/app.js")
    check("app.js is 200", 200, status)
    check("app.js is javascript", "text/javascript; charset=utf-8", head.get("content-type"))
    check("app.js is the repo file", open(os.path.join(ROOT, "dashboard", "app.js"), "rb").read(), script)
    check("app.js is no-store", "no-store", head.get("cache-control"))
    script = script.decode()
    for banned in ["innerHTML", "outerHTML", "insertAdjacentHTML", "document.write", "eval(", "new Function", "setTimeout("]:
        check(f"app.js never uses {banned}", False, banned in script)
    ok("app.js live update is off until toggled", "data-live-toggle" in shell.decode() and "Live update: off" in shell.decode())
    ok("app.js live update polls the health cursor", "/api/health" in script and "setInterval(" in script)
    ok("app.js project switcher is cleared before refill", "select.remove(1)" in script)
    ok("app.js fetches and never writes", "fetch(" in script and not re.search(r"""['"](POST|PUT|PATCH|DELETE)['"]""", script))

    status, head, sheet = daemon.fetch("GET", "/assets/style.css")
    check("style.css is 200", 200, status)
    check("style.css is css", "text/css; charset=utf-8", head.get("content-type"))
    check("style.css is the repo file", open(os.path.join(ROOT, "dashboard", "style.css"), "rb").read(), sheet)
    css_checks(sheet.decode())

    page = shell.decode()
    check("index.html has no inline script", [], re.findall(r"<script(?![^>]*\bsrc=)[^>]*>", page))
    check("index.html has no inline style element", False, "<style" in page)
    check("index.html has no style attribute", False, bool(re.search(r"\sstyle\s*=", page)))
    check("index.html has no event handler attribute", False, bool(re.search(r"\son[a-z]+\s*=", page)))
    check("index.html loads only /assets files", ["/assets/app.js", "/assets/style.css"],
          sorted(re.findall(r"(?:src|href)=\"(/assets/[^\"]+)\"", page)))


def cursor_checks(scratch):
    """Live update polls /api/health: its cursor must move on a write."""
    db = os.path.join(scratch, "live")
    cli(db, "--actor", "manager", "add", "first")
    daemon = Daemon(db)
    try:
        before = json.loads(daemon.request("GET", "/api/health")[1])["cursor"]
        cli(db, "--actor", "manager", "add", "second")
        after = json.loads(daemon.request("GET", "/api/health")[1])["cursor"]
        ok("health cursor moves after a write", before != after)
        check("health cursor is stable without a write", after, json.loads(daemon.request("GET", "/api/health")[1])["cursor"])
    finally:
        daemon.close()


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
        check("health refuses HEAD", 405, daemon.request("HEAD", "/api/health")[0])
        check("unknown path is 404", 404, daemon.request("GET", "/nope")[0])
        ok("health carries the board cursor", health["cursor"].startswith(health["instance"]))
        ok("health carries map usage", isinstance(health["map"]["used_bytes"], int) and isinstance(health["map"]["size_bytes"], int) and 0 < health["map"]["used_bytes"] <= health["map"]["size_bytes"])
        static_checks(daemon)

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
        check("unknown MCP query matches API query refusal", 400, daemon.rpc("/mcp/alice?foo=1", "tools/list")[0])

        # reset against a resident daemon is the busy refusal.
        before_grow = int(daemon.call("/mcp/alice", "task_list").split("total ")[1].split()[0])
        grow = cli(db, "grow", "--map-size-mb", "4096")
        check("grow against the daemon's board exits 1", 1, grow.returncode)
        has("grow names the busy refusal", "another client is attached", grow.stderr)
        check("the board survived the refused grow", before_grow, int(daemon.call("/mcp/alice", "task_list").split("total ")[1].split()[0]))
        reset = cli(db, "reset", "--yes")
        check("reset against the daemon's board exits 1", 1, reset.returncode)
        has("reset names the busy refusal", "another client is attached", reset.stderr)
        check("the board survived the reset", 3, int(daemon.call("/mcp/alice", "task_list").split("total ")[1].split()[0]))
    finally:
        daemon.close()

    usage_db = os.path.join(scratch, "usage-board")
    usage_daemon = Daemon(usage_db)
    try:
        before = json.loads(usage_daemon.request("GET", "/api/health")[1])["map"]["used_bytes"]
        cli_env = os.environ.copy()
        cli_env["AGENTBOARD_ACTOR"] = "manager"
        large_description = "x" * 30000
        for index in range(20):
            subprocess.check_call([BIN, "--db", usage_db, "add", f"large {index}", "--description", large_description], stdout=subprocess.DEVNULL, env=cli_env)
        health_after = json.loads(usage_daemon.request("GET", "/api/health")[1])["map"]["used_bytes"]
        ok("health map usage observes cross-process writes", health_after > before)
        brief = subprocess.check_output([BIN, "--db", usage_db, "brief"], text=True, env=cli_env)
        brief_used = int(re.search(r"^map used [0-9]+\.[0-9]% \(([0-9]+) of [0-9]+ bytes\)$", brief, re.M).group(1))
        check("brief and health map bytes match", health_after, brief_used)
    finally:
        usage_daemon.close()

    # Non-loopback bind needs the flag.
    refused = subprocess.run([BIN, "--db", db, "daemon", "192.0.2.1:0"], capture_output=True, text=True, timeout=30)
    check("non-loopback bind without the flag exits 1", 1, refused.returncode)
    has("non-loopback refusal names the flag", "--allow-non-loopback", refused.stderr)

    # A wildcard bind is refused with or without the flag: the Host guard would 421
    # every remote client, so the daemon asks for one named address instead.
    for wildcard, shown in (("0.0.0.0:0", "0.0.0.0"), ("[::]:0", "::")):
        for extra in ([], ["--allow-non-loopback"]):
            refused = subprocess.run(
                [BIN, "--db", db, "daemon", wildcard, *extra], capture_output=True, text=True, timeout=30
            )
            label = f"wildcard bind {wildcard} {' '.join(extra) or 'without the flag'}"
            check(f"{label} exits 1", 1, refused.returncode)
            has(f"{label} names the unspecified address", f"cannot bind the unspecified address {shown}", refused.stderr)
            check(f"{label} opens no listener", "", refused.stdout)

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

    api_checks(scratch)
    cursor_checks(scratch)

    print(f"daemon: {passed}/{passed + failed} passed (http, lmdb)")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
