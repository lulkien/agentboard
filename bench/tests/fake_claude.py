#!/usr/bin/env python3
"""Canned stream-json endpoint for benchmark tests; makes no network calls."""
import json
import os
from pathlib import Path

assert os.environ.get("AGENTBOARD_DB", "").startswith("/tmp/") or os.environ.get("AGENTBOARD_DB", "").startswith("/var/folders/")
assert Path(os.environ["AGENTBOARD_DB"]).is_dir()
assert os.environ.get("BENCH_MCP_CONFIG") and Path(os.environ["BENCH_MCP_CONFIG"]).is_file()
assert int(os.environ["BENCH_SCRATCH_PORT"]) > 0

arm = os.environ.get("BENCH_TEST_ARM", "A-sonnet")
scenario = os.environ.get("BENCH_TEST_SCENARIO", "normal")
counter = Path(os.environ["AGENTBOARD_DB"]).parent / "fake-counter"
index = int(counter.read_text()) if counter.exists() else 0
counter.write_text(str(index + 1))
allowed = ["agentboard"] if arm == "B-team" else []
if os.environ.get("BENCH_TEST_BAD_MCP") == "1":
    allowed = ["unexpected"]
print(json.dumps({"type": "system", "subtype": "init", "mcp_servers": allowed,
                  "tools": [{"name": "mcp__agentboard__task_get", "server": "agentboard"}] if allowed else []}))
print(json.dumps({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "mcp__agentboard__task_get"}]}}))
print(json.dumps({"type": "result", "usage": {"input_tokens": 10, "output_tokens": 4,
                  "cache_creation_input_tokens": 2, "cache_read_input_tokens": 3,
                  "duration_ms": 20, "num_turns": 1}, "total_cost_usd": 0.01}))
log = Path(os.environ["AGENTBOARD_DB"]).parent / "session-env.jsonl"
with log.open("a") as stream:
    stream.write(json.dumps({key: os.environ.get(key) for key in
                             ("AGENTBOARD_DB", "BENCH_MCP_CONFIG", "BENCH_SCRATCH_PORT")}) + "\n")
if not os.environ.get("BENCH_TEST_NO_BOARD_MUTATION"):
    binary = os.environ.get("BENCH_AGENTBOARD", "agentboard")
    get = __import__("subprocess").run([binary, "get", os.environ["BENCH_ROW_ID"]], env=os.environ,
                                       text=True, capture_output=True, check=True).stdout
    revision = next(int(line.split()[1]) for line in get.splitlines() if line.startswith("revision "))
    transitions = {
        "normal": [("implementing", "coder"), ("reviewing", "reviewer"), ("done", None)],
        "split": [("todo", "manager")], "blocked": [("blocked", None)],
        "stalled": [], "cap": [("implementing", "coder"), ("reviewing", "reviewer")],
        "reopen": [("implementing", "coder"), ("reviewing", "reviewer"),
                   ("implementing", "coder"), ("done", None)],
    }
    transition = transitions[scenario]
    if transition:
        status, owner = transition[index % len(transition)]
        args = [binary, "update", os.environ["BENCH_ROW_ID"], "--if-revision", str(revision),
                "--status", status, "--force", "--reason", "stub transition", "--new-actor"]
        if owner:
            args += ["--assignee", owner]
        __import__("subprocess").run(args, env=os.environ, check=True, capture_output=True)
