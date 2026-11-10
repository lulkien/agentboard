#!/usr/bin/env python3
"""End-to-end proof for the optional read-only dashboard."""

from __future__ import annotations

import os
import html as html_lib
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from urllib.parse import urlencode


ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("AGENTBOARD_TEST_BIN", ROOT / "target/debug/agentboard"))
PAGE_SIZE = 20
CSP = "default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
READ_ONLY_DB = None
BASELINE = os.environ.get("AGENTBOARD_DASHBOARD_BASELINE")


def run_cli(db: Path, *args: str) -> str:
    env = os.environ.copy()
    env.pop("AGENTBOARD_DB", None)
    env["AGENTBOARD_ACTOR"] = "manager"
    result = subprocess.run(
        [str(BIN), *args, "--db", str(db)],
        cwd=ROOT,
        env=env,
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def request(port: int, method: str, target: str) -> tuple[int, str, dict[str, str]]:
    before = feed_cursor(READ_ONLY_DB) if method == "GET" and READ_ONLY_DB else None
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}{target}", method=method
    )
    try:
        with urllib.request.urlopen(req, timeout=4) as response:
            result = response.status, response.read().decode(), {k.lower(): v for k, v in response.headers.items()}
    except urllib.error.HTTPError as response:
        result = response.code, response.read().decode(), {k.lower(): v for k, v in response.headers.items()}
    if before is not None:
        assert feed_cursor(READ_ONLY_DB) == before, f"GET {target} changed the board event cursor"
    return result


def raw_request(
    port: int, method: str, target: str, host: str | None
) -> tuple[int, str, dict[str, str]]:
    host_line = f"Host: {host}\r\n" if host is not None else ""
    with socket.create_connection(("127.0.0.1", port), timeout=4) as connection:
        connection.sendall(
            f"{method} {target} HTTP/1.1\r\n{host_line}Connection: close\r\n\r\n".encode()
        )
        response = bytearray()
        while chunk := connection.recv(4096):
            response.extend(chunk)
    headers, body = bytes(response).split(b"\r\n\r\n", 1)
    lines = headers.decode().split("\r\n")
    status = int(lines[0].split()[1])
    parsed_headers = {
        name.lower(): value.strip()
        for line in lines[1:]
        if ":" in line
        for name, value in [line.split(":", 1)]
    }
    return status, body.decode(), parsed_headers


def compare_baseline(db: Path, current_port: int, baseline: str) -> None:
    baseline_bin = Path(baseline)
    if not baseline_bin.is_absolute():
        baseline_bin = ROOT / baseline_bin
    if not baseline_bin.is_file():
        raise AssertionError(f"dashboard baseline binary not found: {baseline_bin}")

    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        baseline_port = probe.getsockname()[1]
    env = os.environ.copy()
    env.pop("AGENTBOARD_DB", None)
    server = subprocess.Popen(
        [str(baseline_bin), "dashboard", f"127.0.0.1:{baseline_port}", "--db", str(db)],
        cwd=ROOT,
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        for _ in range(50):
            if server.poll() is not None:
                raise AssertionError(f"baseline dashboard exited early with code {server.returncode}")
            try:
                request(baseline_port, "GET", "/")
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise AssertionError("baseline dashboard did not accept connections")

        feed_status, feed_body, _headers = request(
            current_port, "GET", "/feed?task=3&kind=commented"
        )
        assert feed_status == 200, "filtered feed failed before baseline comparison"
        older = re.search(
            r'href="(/feed\?task=3&amp;kind=commented&amp;before=[^"]+)"', feed_body
        )
        assert older, "older feed link missing before baseline comparison"
        older_feed = older.group(1).replace("&amp;", "&")

        pages = [
            "/",
            "/epics",
            "/tasks",
            "/tasks?status=todo",
            "/tasks?offset=20",
            "/tasks?parent=1",
            "/tasks?assignee=alice%3C%26",
            "/tasks/1",
            "/tasks/3",
            "/tasks/2",
            "/feed",
            "/feed?task=3&kind=commented",
            older_feed,
        ]
        targets = pages + [
            f"{page}{'&' if '?' in page else '?'}project=1" for page in pages
        ]
        targets.extend(
            ["/tasks?project=0", "/elsewhere", "/assets/dashboard.css", "/assets/theme.js"]
        )
        assert len(targets) == 30, f"baseline URL list changed: expected 30, got {len(targets)}"

        for target in targets:
            actual_status, actual_body, actual_headers = request(current_port, "GET", target)
            baseline_status, baseline_body, baseline_headers = request(
                baseline_port, "GET", target
            )
            if actual_status != baseline_status:
                raise AssertionError(
                    f"baseline mismatch for {target}: status {actual_status} != {baseline_status}"
                )
            if actual_body.encode() != baseline_body.encode():
                raise AssertionError(f"baseline mismatch for {target}: response body differs")
            for header in ("content-type", "content-security-policy"):
                if actual_headers.get(header) != baseline_headers.get(header):
                    raise AssertionError(
                        f"baseline mismatch for {target}: {header} differs "
                        f"({actual_headers.get(header)!r} != {baseline_headers.get(header)!r})"
                    )
    finally:
        server.terminate()
        try:
            server.wait(timeout=3)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait(timeout=3)

    print("dashboard: 30 pages byte-identical to baseline")


def feed_cursor(db: Path) -> str:
    output = run_cli(db, "feed", "--after", "0")
    match = re.search(r"^cursor (.+)$", output, re.MULTILINE)
    assert match, f"feed cursor missing from output: {output}"
    return match.group(1)


def main() -> int:
    global READ_ONLY_DB
    if not BIN.is_file():
        raise SystemExit(f"feature binary not found: {BIN}; build with --features dashboard")

    with tempfile.TemporaryDirectory(prefix="agentboard-dashboard-") as temp:
        db = Path(temp) / "board"
        READ_ONLY_DB = db
        run_cli(db, "project", "add", "--name", "dashboard", "--path", "/dashboard")
        long_unbroken_title = "T90" + "x" * 195 + "<&"
        escaped_unbroken_title = html_lib.escape(long_unbroken_title, quote=True)
        escaped_dangerous_title = "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;"
        escaped_epic_title = "Epic <script>alert('epic')</script>"
        escaped_epic_title_html = html_lib.escape(escaped_epic_title, quote=True).replace("&#x27;", "&#39;")
        long_spaced_title = "T90 long title " + "words " * 30 + "words"
        run_cli(db, "add", long_unbroken_title)
        run_cli(db, "add", "Done child", "--parent", "1", "--status", "done")
        dangerous_title = "<script>alert('x')</script>"
        description = 'A "quoted" <tag>& first line\nsecond line'
        goal = "Goal <done>&\nkeep this line"
        run_cli(
            db,
            "add",
            dangerous_title,
            "--parent",
            "1",
            "--assignee",
            "alice<&",
            "--new-actor",
            "--project",
            "1",
            "--description",
            description,
            "--goal",
            goal,
        )
        run_cli(db, "update", "3", "--if-revision", "1", "--priority", "-7")
        coder_report = "Coder <b>&\nsecond line"
        run_cli(db, "update", "3", "--if-revision", "2", "--coder-report", coder_report)
        comment = 'Comment "quoted" <b>&\nsecond line'
        run_cli(db, "comment", "3", "--body", comment)
        run_cli(db, "add", "Planning child", "--parent", "1", "--status", "planning", "--assignee", "manager")
        run_cli(db, "add", "Implementing child", "--parent", "1", "--status", "implementing", "--assignee", "manager")
        run_cli(db, "add", "Blocked child", "--parent", "1", "--status", "blocked", "--assignee", "manager")
        run_cli(db, "add", "Reviewing child", "--parent", "1", "--status", "reviewing")
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        env = os.environ.copy()
        env.pop("AGENTBOARD_DB", None)
        server = subprocess.Popen(
            [str(BIN), "dashboard", f"127.0.0.1:{port}", "--db", str(db)],
            cwd=ROOT,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            for _ in range(50):
                if server.poll() is not None:
                    raise AssertionError(f"dashboard exited early with code {server.returncode}")
                try:
                    status, html, _headers = request(port, "GET", "/")
                    break
                except OSError:
                    time.sleep(0.1)
            else:
                raise AssertionError("dashboard did not accept connections")

            assert status == 200, f"GET / returned {status}"
            cursor_before_host_checks = feed_cursor(db)
            for host in [f"127.0.0.1:{port}", f"localhost:{port}", "127.0.0.1", "localhost"]:
                host_status, _, _headers = raw_request(port, "GET", "/", host)
                assert host_status == 200, f"Host {host} returned {host_status}"
            evil_status, evil_body, _headers = raw_request(port, "GET", "/", "evil.example")
            assert evil_status == 421 and evil_body == "misdirected request", "untrusted Host was not refused with 421"
            missing_status, missing_body, _headers = raw_request(port, "GET", "/", None)
            assert missing_status == 400 and missing_body == "host header required", "missing Host was not refused with 400"
            assert feed_cursor(db) == cursor_before_host_checks, "Host checks changed the board event cursor"
            assert request(port, "GET", "/pulse")[0] == 404, "removed /pulse route did not return 404"
            assert request(port, "GET", "/assets/live.js")[0] == 404, "removed live asset did not return 404"
            for index in range(43):
                run_cli(db, "add", f"Page task {index:02}")
            run_cli(db, "update", "8", "--if-revision", "1", "--title", long_spaced_title)
            run_cli(db, "add", long_unbroken_title)
            run_cli(db, "add", long_spaced_title)
            project_name = '<b>p&"</b>'
            run_cli(db, "project", "add", "--name", project_name, "--path", "/other-dashboard")
            run_cli(db, "add", "Other project row", "--project", "2")
            escaped_epic_id = run_cli(db, "add", escaped_epic_title).split()[1].removeprefix("T")
            run_cli(db, "add", "Escaped epic child", "--parent", escaped_epic_id, "--status", "done")
            run_cli(db, "update", escaped_epic_id, "--if-revision", "1", "--status", "canceled")
            status, html, _headers = request(port, "GET", "/")
            assert status == 200, f"GET / returned {status} after adding fixture rows"
            assert "Needs attention" in html and 'class="active" href="/"' in html, "board home is missing"
            attention = html.split('class="attention panel"', 1)[1].split('</section>', 1)[0]
            assert attention.index("Blocked child") < attention.index("Reviewing child"), "attention order is wrong"
            assert html.index('class="seat-bar') < html.index('class="attention') < html.index('id="col-todo"'), "seat bar, attention rail and kanban columns are out of order"
            assert '<span class="seat-name">manager</span> <span class="seat-count">3</span>' in html, "manager held-task count is missing from seat bar"
            assert '<span class="seat-name">alice&lt;&amp;</span> <span class="seat-count">1</span>' in html, "escaped Alice held-task count is missing from seat bar"
            assert 'href="/tasks?assignee=alice%3C%26"' in html, "Alice seat chip does not link to her filtered task list"
            assert "seat-rack" not in html, "old seat rack is still rendered"
            status, project_two_board, _headers = request(port, "GET", "/?project=2")
            assert status == 200 and '<span class="seat-chip idle" title="manager">' in project_two_board, "unheld manager seat is not idle"
            manager_idle = project_two_board.split('<span class="seat-chip idle" title="manager">', 1)[1].split("</li>", 1)[0]
            assert '<span class="seat-name">manager</span> idle' in manager_idle and 'class="seat-count"' not in manager_idle, "idle manager chip has a count or malformed label"
            project_two_planning = project_two_board.split('id="col-planning"', 1)[1].split('</section>', 1)[0]
            assert '<li class="empty">None</li>' in project_two_planning, "empty planning column has no None state"
            project_one_status, project_one_board, _headers = request(port, "GET", "/?project=1")
            project_one_card3 = re.search(r'<a class="card [^"]+" href="/tasks/3(?:\?[^"]*)?"[^>]*>.*?</a>', project_one_board, re.S).group(0)
            assert project_one_status == 200 and 'class="card-project"' not in project_one_card3, "scoped card still renders a project name"
            column_ids = [html.index(f'id="col-{name}"') for name in ("todo", "planning", "implementing", "blocked", "reviewing")]
            assert column_ids == sorted(column_ids), "kanban columns are not in status order"
            for status_name, expected in (("todo", 48), ("planning", 1), ("implementing", 1), ("blocked", 1), ("reviewing", 1)):
                assert re.search(rf'id="col-{status_name}"><span aria-hidden="true">.</span> {status_name} <span class="column-count">{expected}</span>', html), f"{status_name} column count or glyph is wrong"
            kanban = html.split('class="kanban"', 1)[1].split('class="closed-row"', 1)[0]
            todo_column = kanban.split('id="col-todo"', 1)[1].split('</section>', 1)[0]
            assert todo_column.count('class="card status-todo"') == 8, "todo column does not render exactly eight cards"
            assert 'href="/tasks?status=todo">+40 more</a>' in todo_column, "todo column overflow link is wrong"
            blocked_card = kanban.split('href="/tasks/6"', 1)[1].split('</a>', 1)[0]
            reviewing_card = kanban.split('href="/tasks/7"', 1)[1].split('</a>', 1)[0]
            assert 'class="card status-blocked"' in kanban and '<span class="card-flag">blocked</span>' in blocked_card, "blocked card lacks its visible flag"
            assert '<span class="card-flag">reviewing</span>' in reviewing_card, "reviewing card lacks its visible flag"
            todo_card = todo_column.split('class="card status-todo"', 1)[1].split('</a>', 1)[0]
            assert '<span class="visually-hidden">todo</span>' in todo_card and 'class="card-flag"' not in todo_card, "ordinary todo card has a visible flag or no hidden status"
            card3 = kanban.split('href="/tasks/3"', 1)[1].split('</a>', 1)[0]
            assert f'title="{escaped_dangerous_title}"' in card3 and f'class="card-title">{escaped_dangerous_title}</span>' in card3, "T3 escaped title is missing from the card title and text"
            assert 'alice&lt;&amp;' in card3 and 'in T1' in card3 and 'class="card-project">dashboard</span>' in card3 and '<time datetime="' in card3, "T3 card metadata is incomplete or unescaped"
            assert f'title="{escaped_unbroken_title}"' in html, "full long unbroken title is missing from its card title attribute"
            assert f'title="{long_spaced_title}"' in html, "separate long spaced title fixture is missing from a card title attribute"
            assert 'href="/tasks?status=done"' in html and 'href="/tasks?status=canceled"' in html, "closed row is missing done or canceled links"
            assert "<details" not in html and "status-bay" not in html, "old collapsible status bays remain on the Board"
            status, epics_home, _headers = request(port, "GET", "/epics")
            assert status == 200 and "Epics" in epics_home and 'href="/tasks?parent=1"' in epics_home, "epics route or drill-down link is missing"
            assert f'title="{escaped_unbroken_title}"' in epics_home and f'>{escaped_unbroken_title}</span>' in epics_home, "long unbroken epic title was clipped or escaped incorrectly"
            assert f'title="{escaped_epic_title_html}"' in epics_home and f'class="strip-title">{escaped_epic_title_html}</span>' in epics_home, "independent script-like epic title is not escaped in both title and text"
            assert "1 / 6 done" in epics_home, "epic direct-child progress is wrong"
            assert 'role="img" aria-label="1 of 6 direct children done"' in epics_home, "epic rollup is not accessible"
            assert epics_home.count("<rect ") == 7, "epic rollup did not include each populated status"
            epic_row = epics_home.split('class="epic-row depth-0"', 1)[1].split("</article>", 1)[0]
            child_strip_list = epic_row.split('class="strip-list epic-children"', 1)[1].split("</ul>", 1)[0]
            assert child_strip_list.count('class="task-strip ') == 4, "epic did not render its capped direct child strips"
            assert ">+2 more</a>" in epic_row and ">All children</a>" in epic_row, "epic child overflow or all-children link is missing"
            assert 'class="active" href="/epics"' in epics_home, "epics tab was not marked active"
            assert "/assets/dashboard.css" in html and "/assets/theme.js" in html and "live.js" not in html, "embedded dashboard assets are wrong"
            header = html.split("<header", 1)[1].split("</header>", 1)[0]
            assert header.index("Agentboard") < header.index(">Board</a>") < header.index(">Epics</a>") < header.index(">Tasks</a>") < header.index(">Feed</a>") < header.index("project-switcher"), "header order is wrong"
            assert '<select class="header-control" name="project" aria-label="Project">' in header and ">Show</button>" in header, "project switcher is missing its accessible select or Show button"
            assert "Change theme" not in header and ">Project<" not in header and "<label" not in header, "header contains extra visible controls or a Project label"
            assert html.count("<footer>") == 1 and html.index("</main>") < html.index("<footer>") and "<footer><button class=\"header-control theme-toggle\"" in html, "footer theme control is missing or misplaced"
            footer = html.split("<footer>", 1)[1].split("</footer>", 1)[0]
            assert re.search(r'<button[^>]*>Change theme</button>', footer) and "current theme" in request(port, "GET", "/assets/theme.js")[1], "theme button label or mode accessibility is missing"
            dashboard_css = request(port, "GET", "/assets/dashboard.css")[1]
            assert re.search(r"\.seat-name\s*\{[^}]*text-overflow:ellipsis[^}]*min-width:0", dashboard_css), "seat names can overflow their chips"
            assert re.search(r"\.kanban\s*\{[^}]*grid-template-columns:repeat\(5,minmax\(0,1fr\)\)", dashboard_css), "kanban does not use five shrinkable columns"
            assert re.search(r"\.card-title\s*\{[^}]*-webkit-line-clamp:3[^}]*overflow:hidden[^}]*overflow-wrap:anywhere[^}]*max-height:", dashboard_css), "card titles lack the three-line clamp and long-token fallback"
            assert re.search(r"(?m)^\.card\s*\{[^}]*min-width:0", dashboard_css), "cards keep an intrinsic minimum width"
            assert re.search(r"\.column-head\s*\{[^}]*position:sticky", dashboard_css), "column headings are not sticky"
            assert re.search(r"@media\s*\(max-width:899px\)\s*\{[^}]*\.kanban\s*\{[^}]*grid-template-columns:1fr", dashboard_css), "kanban does not stack on narrow screens"
            assert re.search(r"\.attention \.strip-list\s*\{[^}]*grid-template-columns:repeat\(2,minmax\(0,1fr\)\)", dashboard_css), "attention rail is not a two-column strip grid"
            assert re.search(r"@media\s*\(max-width:899px\)\s*\{[^}]*\.attention \.strip-list\s*\{[^}]*grid-template-columns:1fr", dashboard_css), "attention rail does not collapse to one column on narrow screens"
            assert re.search(r"\.topbar\s*\{[^}]*position:sticky[^}]*top:0[^}]*z-index:\d+[^}]*background:var\(--strip\)", dashboard_css), "top bar is not sticky, layered and opaque"
            assert "scroll-padding-top:70px" in dashboard_css and "scroll-margin-top:70px" in dashboard_css, "anchors do not account for the sticky header"
            assert re.search(r"\.header-control\s*\{[^}]*min-height:40px[^}]*padding:\.5rem \.75rem[^}]*font:inherit", dashboard_css), "header control sizing is not shared"
            assert ".topbar {" in dashboard_css and "flex-wrap:wrap" in dashboard_css, "header does not wrap at narrow widths"
            assert ".project-switcher button[type=submit] { padding:.5rem .75rem; }" in dashboard_css, "Show button does not share the theme control padding"
            assert "body { min-height:100vh; display:flex; flex-direction:column; }" in dashboard_css and "main { flex:1;" in dashboard_css, "page layout does not keep the footer at the viewport bottom"
            assert "prefers-color-scheme" in dashboard_css, "system theme defaults are missing"
            assert "pre { font:inherit; }" in dashboard_css, "feed change details do not inherit the dashboard font"
            assert "prefers-reduced-motion" in dashboard_css and ":focus-visible" in dashboard_css, "reduced motion or keyboard focus style is missing"
            theme_status, theme_js, _headers = request(port, "GET", "/assets/theme.js")
            assert theme_status == 200 and "localStorage" in theme_js and "try" in theme_js, "theme persistence/failure handling is missing"
            assert "data-theme-toggle" in theme_js and "setAttribute('data-theme'" in theme_js and "removeAttribute('data-theme')" in theme_js and "button.textContent = 'Change theme'" in theme_js, "theme modes or visible button label are missing"
            for path in ["/", "/epics", "/tasks", "/tasks/1", "/feed"]:
                page_status, body, headers = request(port, "GET", path)
                assert page_status == 200, f"GET {path} returned {page_status}"
                assert all(term not in body for term in ("pulse", "Follow", "live.js", "data-cursor")), f"{path} contains removed live UI"
                assert body.count("<footer>") == 1 and body.index("</main>") < body.index("<footer>"), f"{path} footer is missing or misplaced"
                page_footer = body.split("<footer>", 1)[1].split("</footer>", 1)[0]
                assert re.search(r'<button[^>]*>Change theme</button>', page_footer), f"{path} footer theme button is missing"
                assert headers.get("content-security-policy") == CSP, f"{path} has wrong CSP"
                assert headers.get("cache-control") == "no-store", f"{path} is cacheable"
                assert headers.get("x-content-type-options") == "nosniff", f"{path} lacks nosniff"
                assert headers.get("referrer-policy") == "no-referrer", f"{path} has wrong referrer policy"
                assert 'style="' not in body and "<style" not in body, f"{path} has inline CSS"
                assert not re.search(r"<script(?![^>]*\bsrc=)", body), f"{path} has inline script"
                assert 'href="#main"' in body and 'id="main"' in body, f"{path} lacks skip link"
                assert 'aria-current="page"' in body, f"{path} lacks current nav state"
                badges = re.findall(r'<span class="badge status-([a-z]+)">((?:<span[^>]*>[^<]*</span> )?[^<]*)</span>', body)
                assert len(badges) == body.count('class="badge status-'), f"{path} has malformed status badge"
                for status_name, badge in badges:
                    assert status_name in badge, f"{path} badge lacks {status_name} text"
            for path in ["/", "/tasks", "/epics", "/tasks/3", "/feed"]:
                scoped_status, scoped, _headers = request(port, "GET", f"{path}?project=1")
                assert scoped_status == 200, f"{path}?project=1 returned {scoped_status}"
                links = [html_lib.unescape(href) for href in re.findall(r'href="(/[^"]+)"', scoped)]
                assert all("project=1" in href for href in links if not href.startswith("/assets/")), f"{path} lost project scope: {links}"
                assert '<option value="1" selected>' in scoped, f"{path} did not select project 1"
            status, second_project, _headers = request(port, "GET", "/tasks?project=2")
            assert status == 200 and "Other project row" in second_project and "Page task" not in second_project and "&lt;script&gt;alert" not in second_project, "project 2 task list leaked another project"
            assert "&lt;b&gt;p&amp;&quot;&lt;/b&gt;" in second_project, "project name was not escaped in switcher"
            for path, expected in [("/tasks?project=99", 404), ("/tasks?project=0", 400), ("/tasks?project=1&project=2", 400), ("/tasks/3?status=todo", 400)]:
                actual, _, _headers = request(port, "GET", path)
                assert actual == expected, f"{path} returned {actual}, expected {expected}"
            for path in ["/epics", "/tasks", "/tasks/3", "/feed"]:
                plain = request(port, "GET", path)
                blank = request(port, "GET", f"{path}?project=")
                assert blank[:2] == plain[:2], f"blank project changed {path}"
            for asset in ["/assets/dashboard.css", "/assets/theme.js"]:
                asset_status, _, headers = request(port, "GET", asset)
                assert asset_status == 200 and headers.get("x-content-type-options") == "nosniff", f"{asset} lacks nosniff"

            status, tasks, _headers = request(port, "GET", "/tasks")
            assert status == 200 and 'class="active" href="/tasks"' in tasks, "tasks tab is missing or inactive"
            assert escaped_unbroken_title in tasks and long_spaced_title in tasks, "long titles are missing from the tasks table"
            assert re.search(r"\.strip-list\s*>\s*li\s*\{[^}]*min-width:0", dashboard_css), "strip list items keep their intrinsic min-content width"
            assert re.search(r"\.strip-title\s*\{[^}]*overflow-wrap:anywhere", dashboard_css), "strip titles do not permit long words to shrink"
            assert re.search(r"\.issue-header h2\s*\{[^}]*overflow-wrap:anywhere", dashboard_css), "unbroken detail titles can overflow their page"
            chips = re.findall(r'<a class="status-chip(?: status-[a-z]+)?"[^>]*>([^<]+)</a>', tasks)
            assert chips == ["All 55", "todo 48", "planning 1", "implementing 1", "blocked 1", "reviewing 1", "done 2", "canceled 1"], f"task status counts are wrong: {chips}"
            status, active_todo, _headers = request(port, "GET", "/tasks?status=todo")
            assert status == 200 and 'class="status-chip status-todo" href="/tasks?status=todo" aria-current="true"' in active_todo, "active status chip is missing"
            pager = re.search(r'<nav class="pagination"[^>]*>(.*?)</nav>', tasks)
            assert pager and '<span class="page-arrow" aria-label="Previous page" aria-disabled="true">←</span><span>Page 1 of 3</span>' in pager.group(1), "first page indicator or disabled previous arrow is wrong"
            assert '<a class="page-arrow" aria-label="Next page" rel="next" href="/tasks?offset=20">→</a>' in pager.group(1), "first page next arrow is missing or malformed"
            second_status, second_page, _headers = request(port, "GET", "/tasks?offset=20")
            second_pager = re.search(r'<nav class="pagination"[^>]*>(.*?)</nav>', second_page)
            assert second_status == 200 and second_pager and 'aria-label="Previous page" rel="prev" href="/tasks?offset=0"' in second_pager.group(1) and 'Page 2 of 3' in second_pager.group(1) and 'aria-label="Next page" rel="next" href="/tasks?offset=40"' in second_pager.group(1), "middle page arrows or indicator are wrong"
            last_status, last_page, _headers = request(port, "GET", "/tasks?offset=40")
            last_pager = re.search(r'<nav class="pagination"[^>]*>(.*?)</nav>', last_page)
            assert last_status == 200 and last_pager and 'Page 3 of 3' in last_pager.group(1) and '<span class="page-arrow" aria-label="Next page" aria-disabled="true">→</span>' in last_pager.group(1), "last page does not disable next arrow"
            assert escaped_dangerous_title in last_page, "independent escaped script title is missing from the tasks table"
            assert "Previous</a>" not in tasks and "Next</a>" not in tasks, "pager still renders text links"
            first_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', tasks)]
            assert first_ids == list(range(55, 35, -1)), f"task page 1 is not strictly newest-first: {first_ids}"

            status, detail, _headers = request(port, "GET", "/tasks/3")
            assert status == 200, f"task detail returned {status}"
            status, long_detail, _headers = request(port, "GET", "/tasks/51")
            assert status == 200 and escaped_unbroken_title in long_detail, "long unbroken title is missing from task detail"
            status, spaced_detail, _headers = request(port, "GET", "/tasks/52")
            assert status == 200 and long_spaced_title in spaced_detail, "long spaced title is missing from task detail"
            assert 'class="active" href="/tasks"' in detail, "task detail did not mark the Tasks tab active"
            assert "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;" in detail, "detail title was not escaped"
            assert "A &quot;quoted&quot; &lt;tag&gt;&amp; first line\nsecond line" in detail, "full multiline description was not escaped/preserved"
            assert "Goal &lt;done&gt;&amp;\nkeep this line" in detail, "full multiline goal was not escaped/preserved"
            detail_ids = [detail.index(f'id="{section}"') for section in ("goal", "description", "coder-report", "activity")]
            assert detail_ids == sorted(detail_ids), "detail sections are not in goal-first reading order"
            assert 'href="#goal"' in detail and 'href="#description"' in detail and 'href="#coder-report"' in detail and 'href="#activity"' in detail, "On this page links are missing"
            assert "alice&lt;&amp;" in detail and "1" in detail, "detail metadata was not escaped"
            assert "Reporter" in detail and "manager" in detail and "Priority" in detail and "-7" in detail, "reporter or signed priority metadata is missing"
            assert 'href="/tasks/1"' in detail and "Parent" in detail, "parent link is missing"
            assert "updated" in detail and "commented" in detail and "<time datetime=" in detail, "activity event kind or timestamp is missing"
            assert "Comment &quot;quoted&quot; &lt;b&gt;&amp;\nsecond line" in detail, "comment text was not escaped/preserved"
            assert "Coder report" in detail and "Coder &lt;b&gt;&amp;\nsecond line" in detail, "report section was not escaped/preserved"
            assert "Analyst report" not in detail and "Reviewer report" not in detail, "empty report sections were printed"
            assert 'id="analyst-report"' not in detail and 'id="reviewer-report"' not in detail, "empty report anchors were printed"
            assert '<ul class="change-list">' in detail and "<strong>priority</strong>" in detail, "update changes were not rendered"
            assert 'href="/feed?task=3"' in detail, "older activity link is missing"

            status, epic_detail, _headers = request(port, "GET", "/tasks/1")
            assert status == 200 and "Child tasks" in epic_detail, "epic detail child list is missing"
            assert "1 / 6 done" in epic_detail and 'max="6" value="1"' in epic_detail, "epic detail rollup is wrong"
            assert "Planning child" in epic_detail and 'class="task-strip status-planning"' in epic_detail, "direct child strip is missing from epic detail"
            status, leaf_detail, _headers = request(port, "GET", "/tasks/2")
            assert status == 200 and "Child tasks" not in leaf_detail, "leaf was incorrectly treated as an epic"
            for path, expected in [("/tasks/99999", 404), ("/tasks/0", 400), ("/tasks/nope", 400), ("/tasks/3/extra", 404)]:
                detail_status, _, _headers = request(port, "GET", path)
                assert detail_status == expected, f"{path} returned {detail_status}, expected {expected}"
            detail_status, _, _headers = request(port, "GET", "/tasks/3?status=todo")
            assert detail_status == 400, "detail route accepted a query parameter"

            for index in range(45):
                run_cli(db, "comment", "3", "--body", f"feed note {index:02}")
            run_cli(db, "comment", "53", "--body", "foreign feed marker")
            before = feed_cursor(db)
            scoped_feed_status, scoped_feed, _headers = request(port, "GET", "/feed?project=1")
            assert scoped_feed_status == 200 and "foreign feed marker" not in scoped_feed and 'href="/tasks/53?project=1"' not in scoped_feed, "scoped feed leaked project 2"
            scoped_older = re.search(r'href="(/feed\?before=[^"]+project=1)"', scoped_feed)
            assert scoped_older, "scoped feed lost project on older link"
            older_status, older_scoped, _headers = request(port, "GET", html_lib.unescape(scoped_older.group(1)))
            assert older_status == 200 and 'href="/tasks/53?project=1"' not in older_scoped, "older scoped feed leaked project 2"

            status, second_page, _headers = request(port, "GET", "/tasks?offset=20")
            assert status == 200 and "Page task 20" in second_page, "offset paging did not advance"
            assert "Page task 00" not in second_page, "offset page repeats the first page"
            second_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', second_page)]
            assert second_ids == list(range(35, 15, -1)), f"task page 2 has a gap, duplicate, or wrong order: {second_ids}"
            assert not set(first_ids) & set(second_ids) and first_ids[-1] - 1 == second_ids[0], "task pages overlap or have a gap"

            query = urlencode({"assignee": "alice<&"})
            status, filtered, _headers = request(port, "GET", f"/tasks?{query}")
            assert status == 200 and dangerous_title not in filtered, "filter returned unsafe raw title"
            assert "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;" in filtered, "assignee filter missed the task"
            assert "<script>alert('x')</script>" not in filtered, "raw task markup was rendered"
            assert "alice&lt;&amp;" in filtered and "1" in filtered, "task metadata was not escaped"
            assert 'href="/tasks/3"' in filtered, "task detail link is missing"
            status, filtered_done, _headers = request(port, "GET", "/tasks?status=done")
            assert status == 200 and "Done child" in filtered_done, "status filter missed done task"
            assert "Planning child" not in filtered_done, "status filter leaked another status"
            filtered_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', filtered_done)]
            assert filtered_ids == sorted(filtered_ids, reverse=True), f"filtered tasks are not newest-first: {filtered_ids}"
            status, invalid, _headers = request(port, "GET", "/tasks?status=unknown")
            assert status == 400 and "invalid dashboard query" in invalid, "invalid status was accepted"
            status, not_found, _headers = request(port, "GET", "/elsewhere")
            assert status == 404, f"unknown path returned {status}"
            status, child_tasks, _headers = request(port, "GET", "/tasks?parent=1")
            assert status == 200 and "Done child" in child_tasks and "Page task 00" not in child_tasks, "parent filter failed"
            child_chips = re.findall(r'<a class="status-chip(?: status-[a-z]+)?"[^>]*>([^<]+)</a>', child_tasks)
            assert child_chips[0] == "All 6" and sum(int(chip.rsplit(" ", 1)[1]) for chip in child_chips[1:]) == 6, f"parent status chips are wrong: {child_chips}"
            child_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', child_tasks)]
            assert len(child_ids) > 1 and child_ids == sorted(child_ids, reverse=True), f"multi-row parent filter is not newest-first: {child_ids}"
            status, all_tasks, _headers = request(port, "GET", "/tasks")
            assert status == 200, "unfiltered tasks failed"
            status, todo_tasks, _headers = request(port, "GET", "/tasks?status=todo")
            assert status == 200, "todo task filter failed"
            for query, expected_tasks in [("status=todo&assignee=&parent=", todo_tasks), ("status=&assignee=&parent=", all_tasks)]:
                blank_status, blank_tasks, _headers = request(port, "GET", f"/tasks?{query}")
                assert blank_status == 200 and blank_tasks == expected_tasks, f"blank task filters {query} changed results"
            status, blank_assignee_tasks, _headers = request(port, "GET", "/tasks?assignee=")
            assert status == 200 and blank_assignee_tasks == all_tasks, "blank assignee filter changed results"
            for bad_query in ["status=todo&status=done", "unknown=1", "parent=0", "status=%"]:
                status, _, _headers = request(port, "GET", f"/tasks?{bad_query}")
                assert status == 400, f"malformed query {bad_query} returned {status}"

            status, feed_page_one, _headers = request(port, "GET", "/feed?task=3&kind=commented")
            assert status == 200 and 'class="active" href="/feed"' in feed_page_one, "feed tab is missing or inactive"
            notes_one = re.findall(r"feed note (\d{2})", feed_page_one)
            assert notes_one == [f"{index:02}" for index in range(25, 45)], f"newest feed page is wrong: {notes_one}"
            older_link = re.search(r'href="(/feed\?task=3&amp;kind=commented&amp;before=[^"]+)"', feed_page_one)
            assert older_link, f"older feed link did not retain filters: {feed_page_one}"
            older_target = older_link.group(1).replace("&amp;", "&")
            status, feed_page_two, _headers = request(port, "GET", older_target)
            notes_two = re.findall(r"feed note (\d{2})", feed_page_two)
            assert status == 200 and notes_two == [f"{index:02}" for index in range(5, 25)], f"second older page has a gap/duplicate: {notes_two}"
            older_link_two = re.search(r'href="(/feed\?task=3&amp;kind=commented&amp;before=[^"]+)"', feed_page_two)
            assert older_link_two, "second older feed link is missing"
            status, feed_page_three, _headers = request(port, "GET", older_link_two.group(1).replace("&amp;", "&"))
            notes_three = re.findall(r"feed note (\d{2})", feed_page_three)
            assert status == 200 and notes_three == [f"{index:02}" for index in range(5)], f"last older page has a gap/duplicate: {notes_three}"
            assert "Comment &quot;quoted&quot; &lt;b&gt;&amp;\nsecond line" in feed_page_three, "feed comment text was not escaped/preserved"
            assert 'href="/feed?task=3&amp;kind=commented"' in feed_page_three, "newest feed link did not retain filters"
            status, updated_feed, _headers = request(port, "GET", "/feed?task=3&kind=updated")
            assert status == 200 and "updated" in updated_feed and "feed note" not in updated_feed, "feed kind filter failed"
            status, all_feed, _headers = request(port, "GET", "/feed")
            assert status == 200, "unfiltered feed failed"
            status, task_feed, _headers = request(port, "GET", "/feed?task=3")
            assert status == 200, "task feed filter failed"
            status, kind_feed, _headers = request(port, "GET", "/feed?kind=commented")
            assert status == 200, "kind feed filter failed"
            for query in ["task=&kind=", "task=3&kind=", "task=&kind=commented"]:
                blank_status, blank_feed, _headers = request(port, "GET", f"/feed?{query}")
                expected_feed = task_feed if query == "task=3&kind=" else kind_feed if query == "task=&kind=commented" else all_feed
                assert blank_status == 200 and blank_feed == expected_feed, f"blank feed filters {query} changed results"
            for bad_query in ["task=0", "kind=bad", "task=3&task=4", "before=bad", "before=foreign-board:1", "unknown=x"]:
                status, _, _headers = request(port, "GET", f"/feed?{bad_query}")
                assert status == 400, f"invalid feed query {bad_query} returned {status}"

            for path in ["/", "/epics", "/tasks", "/tasks/1", "/feed", "/assets/dashboard.css", "/assets/theme.js"]:
                status, _, _headers = request(port, "POST", path)
                assert status == 405, f"POST {path} returned {status}"
            assert feed_cursor(db) == before, "POST changed the board event cursor"

            run_cli(db, "add", "Nested child", "--parent", "4")
            nested_status, nested_epics, _headers = request(port, "GET", "/epics")
            assert nested_status == 200 and 'class="epic-row depth-1"' in nested_epics and ">in T1</a>" in nested_epics, "nested epic was not indented under its parent"
            run_cli(db, "add", "Scoped epic", "--project", "1")
            run_cli(db, "add", "Scoped child", "--parent", "57", "--project", "1")
            run_cli(db, "add", "Foreign child", "--parent", "57", "--project", "2")
            scoped_epics_status, scoped_epics, _headers = request(port, "GET", "/epics?project=1")
            assert scoped_epics_status == 200 and "Scoped child" in scoped_epics and "Foreign child" not in scoped_epics, "epic child strips ignored project scope"
            assert 'href="/tasks/58?project=1"' in scoped_epics, "scoped epic child link dropped its project"

            if BASELINE:
                compare_baseline(db, port, BASELINE)

            with socket.create_connection(("127.0.0.1", port), timeout=4) as connection:
                connection.settimeout(4)
                connection.sendall(b"GET / HTTP/1.1\r\nHost: local")
                connection.shutdown(socket.SHUT_WR)
                response = bytearray()
                while True:
                    try:
                        chunk = connection.recv(4096)
                    except ConnectionResetError:
                        break
                    if not chunk:
                        break
                    response.extend(chunk)
            assert response.startswith(b"HTTP/1.1 400 Bad Request"), "incomplete header was not refused"

            with socket.create_connection(("127.0.0.1", port), timeout=4) as connection:
                connection.settimeout(4)
                connection.sendall(b"GET / HTTP/1.1\r\nHost: local\r\nX-Fill: " + b"x" * 9000)
                response = bytearray()
                while True:
                    try:
                        chunk = connection.recv(4096)
                    except ConnectionResetError:
                        break
                    if not chunk:
                        break
                    response.extend(chunk)
            assert response.startswith(b"HTTP/1.1 431 Request Header Fields Too Large"), "oversized header was not refused"

            with socket.create_connection(("127.0.0.1", port), timeout=4) as connection:
                connection.settimeout(4)
                connection.sendall(b"GET / HTTP/1.1\r\nHost: local\r\n")
                response = bytearray()
                while True:
                    try:
                        chunk = connection.recv(4096)
                    except ConnectionResetError:
                        break
                    if not chunk:
                        break
                    response.extend(chunk)
            assert response.startswith(b"HTTP/1.1 408 Request Timeout"), "timed-out header was not refused"
        finally:
            server.terminate()
            try:
                server.wait(timeout=3)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=3)

        refused = subprocess.run(
            [str(BIN), "dashboard", "0.0.0.0:0", "--db", str(db)],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=4,
        )
        refusal = refused.stdout + refused.stderr
        assert refused.returncode == 1 and "--allow-non-loopback" in refusal, "non-loopback bind was not refused with the required flag"

        with socket.socket() as probe:
            probe.bind(("0.0.0.0", 0))
            public_port = probe.getsockname()[1]
        public_server = subprocess.Popen(
            [str(BIN), "dashboard", f"0.0.0.0:{public_port}", "--allow-non-loopback", "--db", str(db)],
            cwd=ROOT,
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            for _ in range(50):
                if public_server.poll() is not None:
                    raise AssertionError(f"dashboard with --allow-non-loopback exited with code {public_server.returncode}")
                try:
                    public_status, _, _headers = raw_request(
                        public_port, "GET", "/", f"127.0.0.1:{public_port}"
                    )
                    break
                except OSError:
                    time.sleep(0.1)
            else:
                raise AssertionError("dashboard with --allow-non-loopback did not bind")
            assert public_status == 200, f"dashboard with --allow-non-loopback returned {public_status}"
        finally:
            public_server.terminate()
            try:
                public_server.wait(timeout=3)
            except subprocess.TimeoutExpired:
                public_server.kill()
                public_server.wait(timeout=3)

    print("dashboard: all checks passed (read-only HTTP, filters, paging, escaping, bounds)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
