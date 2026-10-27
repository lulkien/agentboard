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


def check_counts(html: str) -> None:
    counts = dict(re.findall(r"<li><span>([a-z_]+)</span>: <strong>(\d+)</strong></li>", html))
    expected = {"todo", "planning", "implementing", "blocked", "reviewing", "done", "canceled"}
    assert set(counts) == expected, f"expected all seven status counts; got {counts}"
    assert counts["canceled"] == "0", f"zero count not shown: {counts}"


def main() -> int:
    if not BIN.is_file():
        raise SystemExit(f"feature binary not found: {BIN}; build with --features dashboard")

    with tempfile.TemporaryDirectory(prefix="agentboard-dashboard-") as temp:
        db = Path(temp) / "board"
        run_cli(db, "add", "Epic <script>alert('epic')</script>")
        run_cli(db, "add", "Done child", "--parent", "1", "--status", "done")
        dangerous_title = "<script>alert('x')</script>"
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
        )
        run_cli(db, "add", "Planning child", "--parent", "1", "--status", "planning", "--assignee", "manager")
        run_cli(db, "add", "Implementing child", "--parent", "1", "--status", "implementing", "--assignee", "manager")
        run_cli(db, "add", "Blocked child", "--parent", "1", "--status", "blocked", "--assignee", "manager")
        run_cli(db, "add", "Reviewing child", "--parent", "1", "--status", "reviewing")
        for index in range(45):
            run_cli(db, "add", f"Page task {index:02}")

        before = feed_cursor(db)
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
            assert "&lt;script&gt;alert(&#39;epic&#39;)&lt;/script&gt;" in html, "epic title was not escaped"
            assert "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;" in html, "task title was not escaped"
            assert "<script>alert('x')</script>" not in html, "raw task markup was rendered"
            assert "alice&lt;&amp;" in html, "assignee text was not escaped"
            assert "src/&lt;b&gt;&amp;" in html, "path text was not escaped"
            assert "1/6 done" in html, "epic direct-child progress is wrong"
            assert 'max="6" value="1"' in html, "epic progress bar values are wrong"
            check_counts(html)
            assert 'href="/?offset=20"' in html, "next-page link is missing"

            status, second_page = request(port, "GET", "/?offset=20")
            assert status == 200 and "Page task 20" in second_page, "offset paging did not advance"
            assert "Page task 00" not in second_page, "offset page repeats the first page"

            query = urlencode({"assignee": "alice<&"})
            status, filtered = request(port, "GET", f"/?{query}")
            assert status == 200 and dangerous_title not in filtered, "filter returned unsafe raw title"
            assert "&lt;script&gt;alert(&#39;x&#39;)&lt;/script&gt;" in filtered, "assignee filter missed the task"
            status, filtered_done = request(port, "GET", "/?status=done")
            assert status == 200 and "Done child" in filtered_done, "status filter missed done task"
            assert "Planning child" not in filtered_done, "status filter leaked another status"
            status, invalid = request(port, "GET", "/?status=unknown")
            assert status == 400 and "invalid dashboard query" in invalid, "invalid status was accepted"
            status, not_found = request(port, "GET", "/elsewhere")
            assert status == 404, f"unknown path returned {status}"

            status, _ = request(port, "POST", "/")
            assert status == 405, f"POST returned {status}"
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
