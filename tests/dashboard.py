#!/usr/bin/env python3
"""End-to-end proof for the optional read-only dashboard."""

from __future__ import annotations

import os
from pathlib import Path
import re
import socket
import subprocess
import tempfile
import time
from urllib.parse import urlencode


ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("AGENTBOARD_TEST_BIN", ROOT / "target/debug/agentboard"))
PAGE_SIZE = 20


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


def request(port: int, method: str, target: str) -> tuple[int, str]:
    result = subprocess.run(
        [
            "curl",
            "--silent",
            "--show-error",
            "--max-time",
            "4",
            "-X",
            method,
            "-o",
            "-",
            "-w",
            "\\n%{http_code}",
            f"http://127.0.0.1:{port}{target}",
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode:
        raise ConnectionError(result.stderr.strip() or "curl request failed")
    body, status = result.stdout.rsplit("\n", 1)
    return int(status), body


def feed_cursor(db: Path) -> str:
    output = run_cli(db, "feed", "--after", "0")
    match = re.search(r"^cursor (.+)$", output, re.MULTILINE)
    assert match, f"feed cursor missing from output: {output}"
    return match.group(1)


def main() -> int:
    if not BIN.is_file():
        raise SystemExit(f"feature binary not found: {BIN}; build with --features dashboard")

    with tempfile.TemporaryDirectory(prefix="agentboard-dashboard-") as temp:
        db = Path(temp) / "board"
        run_cli(db, "add", "Epic <script>alert('epic')</script>")
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
            "--path",
            "src/<b>&",
            "--description",
            description,
            "--goal",
            goal,
        )
        run_cli(db, "update", "3", "--if-revision", "1", "--priority", "-7")
        comment = 'Comment "quoted" <b>&\nsecond line'
        run_cli(db, "comment", "3", "--body", comment)
        run_cli(db, "add", "Planning child", "--parent", "1", "--status", "planning", "--assignee", "manager")
        run_cli(db, "add", "Implementing child", "--parent", "1", "--status", "implementing", "--assignee", "manager")
        run_cli(db, "add", "Blocked child", "--parent", "1", "--status", "blocked", "--assignee", "manager")
        run_cli(db, "add", "Reviewing child", "--parent", "1", "--status", "reviewing")
        for index in range(45):
            run_cli(db, "add", f"Page task {index:02}")

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
                    status, html = request(port, "GET", "/")
                    break
                except OSError:
                    time.sleep(0.1)
            else:
                raise AssertionError("dashboard did not accept connections")

            assert status == 200, f"GET / returned {status}"
            assert "Epics" in html and 'href="/tasks?parent=1"' in html, "epics tab or drill-down link is missing"
            assert "&lt;script&gt;alert(&#39;epic&#39;)&lt;/script&gt;" in html, "epic title was not escaped"
            assert "1 / 6 done" in html, "epic direct-child progress is wrong"
            assert 'max="6" value="1"' in html, "epic progress bar values are wrong"
            assert 'class="active" href="/epics"' in html, "epics tab was not marked active"
            assert "/assets/dashboard.css" in html and "/assets/theme.js" in html, "embedded theme assets are missing"
            dashboard_css = request(port, "GET", "/assets/dashboard.css")[1]
            assert "prefers-color-scheme" in dashboard_css, "system theme defaults are missing"
            assert "pre { font:inherit; }" in dashboard_css, "feed change details do not inherit the dashboard font"
            theme_status, theme_js = request(port, "GET", "/assets/theme.js")
            assert theme_status == 200 and "localStorage" in theme_js and "try" in theme_js, "theme persistence/failure handling is missing"

            status, tasks = request(port, "GET", "/tasks")
            assert status == 200 and 'class="active" href="/tasks"' in tasks, "tasks tab is missing or inactive"
            assert 'href="/tasks?offset=20"' in tasks, "next-page link is missing"
            first_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', tasks)]
            assert first_ids == list(range(52, 32, -1)), f"task page 1 is not strictly newest-first: {first_ids}"

            status, detail = request(port, "GET", "/tasks/3")
            assert status == 200, f"task detail returned {status}"
            assert 'class="active" href="/tasks"' in detail, "task detail did not mark the Tasks tab active"
            assert "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;" in detail, "detail title was not escaped"
            assert "A &quot;quoted&quot; &lt;tag&gt;&amp; first line\nsecond line" in detail, "full multiline description was not escaped/preserved"
            assert "Goal &lt;done&gt;&amp;\nkeep this line" in detail, "full multiline goal was not escaped/preserved"
            assert "alice&lt;&amp;" in detail and "src/&lt;b&gt;&amp;" in detail, "detail metadata was not escaped"
            assert "Reporter" in detail and "manager" in detail and "Priority" in detail and "-7" in detail, "reporter or signed priority metadata is missing"
            assert 'href="/tasks/1"' in detail and "Parent" in detail, "parent link is missing"
            assert "updated" in detail and "commented" in detail and "<time datetime=" in detail, "activity event kind or timestamp is missing"
            assert "Comment &quot;quoted&quot; &lt;b&gt;&amp;\nsecond line" in detail, "comment text was not escaped/preserved"
            assert 'href="/feed?task=3"' in detail, "older activity link is missing"

            status, epic_detail = request(port, "GET", "/tasks/1")
            assert status == 200 and "Child tasks" in epic_detail, "epic detail child list is missing"
            assert "1 / 6 done" in epic_detail and 'max="6" value="1"' in epic_detail, "epic detail rollup is wrong"
            assert "Planning child" in epic_detail, "direct child is missing from epic detail"
            status, leaf_detail = request(port, "GET", "/tasks/2")
            assert status == 200 and "Child tasks" not in leaf_detail, "leaf was incorrectly treated as an epic"
            for path, expected in [("/tasks/99999", 404), ("/tasks/0", 400), ("/tasks/nope", 400), ("/tasks/3/extra", 404)]:
                detail_status, _ = request(port, "GET", path)
                assert detail_status == expected, f"{path} returned {detail_status}, expected {expected}"
            detail_status, _ = request(port, "GET", "/tasks/3?status=todo")
            assert detail_status == 400, "detail route accepted a query parameter"

            for index in range(45):
                run_cli(db, "comment", "3", "--body", f"feed note {index:02}")
            before = feed_cursor(db)

            status, second_page = request(port, "GET", "/tasks?offset=20")
            assert status == 200 and "Page task 20" in second_page, "offset paging did not advance"
            assert "Page task 00" not in second_page, "offset page repeats the first page"
            second_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', second_page)]
            assert second_ids == list(range(32, 12, -1)), f"task page 2 has a gap, duplicate, or wrong order: {second_ids}"
            assert not set(first_ids) & set(second_ids) and first_ids[-1] - 1 == second_ids[0], "task pages overlap or have a gap"

            query = urlencode({"assignee": "alice<&"})
            status, filtered = request(port, "GET", f"/tasks?{query}")
            assert status == 200 and dangerous_title not in filtered, "filter returned unsafe raw title"
            assert "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;" in filtered, "assignee filter missed the task"
            assert "<script>alert('x')</script>" not in filtered, "raw task markup was rendered"
            assert "alice&lt;&amp;" in filtered and "src/&lt;b&gt;&amp;" in filtered, "task metadata was not escaped"
            assert 'href="/tasks/3"' in filtered, "task detail link is missing"
            status, filtered_done = request(port, "GET", "/tasks?status=done")
            assert status == 200 and "Done child" in filtered_done, "status filter missed done task"
            assert "Planning child" not in filtered_done, "status filter leaked another status"
            filtered_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', filtered_done)]
            assert filtered_ids == sorted(filtered_ids, reverse=True), f"filtered tasks are not newest-first: {filtered_ids}"
            status, invalid = request(port, "GET", "/tasks?status=unknown")
            assert status == 400 and "invalid dashboard query" in invalid, "invalid status was accepted"
            status, not_found = request(port, "GET", "/elsewhere")
            assert status == 404, f"unknown path returned {status}"
            status, child_tasks = request(port, "GET", "/tasks?parent=1")
            assert status == 200 and "Done child" in child_tasks and "Page task 00" not in child_tasks, "parent filter failed"
            child_ids = [int(task_id) for task_id in re.findall(r'href="/tasks/(\d+)">T\d+</a>', child_tasks)]
            assert len(child_ids) > 1 and child_ids == sorted(child_ids, reverse=True), f"multi-row parent filter is not newest-first: {child_ids}"
            status, all_tasks = request(port, "GET", "/tasks")
            assert status == 200, "unfiltered tasks failed"
            status, todo_tasks = request(port, "GET", "/tasks?status=todo")
            assert status == 200, "todo task filter failed"
            for query, expected_tasks in [("status=todo&assignee=&parent=", todo_tasks), ("status=&assignee=&parent=", all_tasks)]:
                blank_status, blank_tasks = request(port, "GET", f"/tasks?{query}")
                assert blank_status == 200 and blank_tasks == expected_tasks, f"blank task filters {query} changed results"
            status, blank_assignee_tasks = request(port, "GET", "/tasks?assignee=")
            assert status == 200 and blank_assignee_tasks == all_tasks, "blank assignee filter changed results"
            for bad_query in ["status=todo&status=done", "unknown=1", "parent=0", "status=%"]:
                status, _ = request(port, "GET", f"/tasks?{bad_query}")
                assert status == 400, f"malformed query {bad_query} returned {status}"

            status, feed_page_one = request(port, "GET", "/feed?task=3&kind=commented")
            assert status == 200 and 'class="active" href="/feed"' in feed_page_one, "feed tab is missing or inactive"
            notes_one = re.findall(r"feed note (\d{2})", feed_page_one)
            assert notes_one == [f"{index:02}" for index in range(25, 45)], f"newest feed page is wrong: {notes_one}"
            older_link = re.search(r'href="(/feed\?task=3&amp;kind=commented&amp;before=[^"]+)"', feed_page_one)
            assert older_link, f"older feed link did not retain filters: {feed_page_one}"
            older_target = older_link.group(1).replace("&amp;", "&")
            status, feed_page_two = request(port, "GET", older_target)
            notes_two = re.findall(r"feed note (\d{2})", feed_page_two)
            assert status == 200 and notes_two == [f"{index:02}" for index in range(5, 25)], f"second older page has a gap/duplicate: {notes_two}"
            older_link_two = re.search(r'href="(/feed\?task=3&amp;kind=commented&amp;before=[^"]+)"', feed_page_two)
            assert older_link_two, "second older feed link is missing"
            status, feed_page_three = request(port, "GET", older_link_two.group(1).replace("&amp;", "&"))
            notes_three = re.findall(r"feed note (\d{2})", feed_page_three)
            assert status == 200 and notes_three == [f"{index:02}" for index in range(5)], f"last older page has a gap/duplicate: {notes_three}"
            assert "Comment &quot;quoted&quot; &lt;b&gt;&amp;\nsecond line" in feed_page_three, "feed comment text was not escaped/preserved"
            assert 'href="/feed?task=3&amp;kind=commented"' in feed_page_three, "newest feed link did not retain filters"
            status, updated_feed = request(port, "GET", "/feed?task=3&kind=updated")
            assert status == 200 and "updated" in updated_feed and "feed note" not in updated_feed, "feed kind filter failed"
            status, all_feed = request(port, "GET", "/feed")
            assert status == 200, "unfiltered feed failed"
            status, task_feed = request(port, "GET", "/feed?task=3")
            assert status == 200, "task feed filter failed"
            status, kind_feed = request(port, "GET", "/feed?kind=commented")
            assert status == 200, "kind feed filter failed"
            for query in ["task=&kind=", "task=3&kind=", "task=&kind=commented"]:
                blank_status, blank_feed = request(port, "GET", f"/feed?{query}")
                expected_feed = task_feed if query == "task=3&kind=" else kind_feed if query == "task=&kind=commented" else all_feed
                assert blank_status == 200 and blank_feed == expected_feed, f"blank feed filters {query} changed results"
            for bad_query in ["task=0", "kind=bad", "task=3&task=4", "before=bad", "before=foreign-board:1", "unknown=x"]:
                status, _ = request(port, "GET", f"/feed?{bad_query}")
                assert status == 400, f"invalid feed query {bad_query} returned {status}"

            for path in ["/", "/epics", "/tasks", "/tasks/1", "/feed", "/assets/dashboard.css", "/assets/theme.js"]:
                status, _ = request(port, "POST", path)
                assert status == 405, f"POST {path} returned {status}"
            assert feed_cursor(db) == before, "POST changed the board event cursor"

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

    print("dashboard: all checks passed (read-only HTTP, filters, paging, escaping, bounds)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
