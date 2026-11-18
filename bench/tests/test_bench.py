#!/usr/bin/env python3
"""Dependency-free regression proofs for the benchmark runner and renderer."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
spec = importlib.util.spec_from_file_location("bench_run", ROOT / "bench/run.py")
run = importlib.util.module_from_spec(spec); spec.loader.exec_module(run)
spec = importlib.util.spec_from_file_location("bench_report", ROOT / "bench/report.py")
report = importlib.util.module_from_spec(spec); spec.loader.exec_module(report)
checks = 0


def check(condition, message):
    global checks
    checks += 1
    if not condition: raise AssertionError(message)


def test_usage_and_validation():
    events = [{"type":"assistant","message":{"content":[{"type":"tool_use"},{"type":"text"}]}},
              {"type":"result","usage":{"input_tokens":3,"output_tokens":4,"cache_creation_input_tokens":5,"cache_read_input_tokens":6,"duration_ms":7,"num_turns":2},"total_cost_usd":0.25}]
    u = run.usage(events)
    check(u == {"input_tokens":3,"output_tokens":4,"cache_creation_input_tokens":5,"cache_read_input_tokens":6,"total_cost_usd":0.25,"duration_ms":7,"num_turns":2,"tool_calls":1}, "usage sum")
    run.validate_init([{"type":"system","subtype":"init","mcp_servers":["agentboard"],"tools":[{"server":"agentboard"}]}], "B-team")
    run.validate_init([{"type":"system","subtype":"init","mcp_servers":[{"name":"agentboard"}],"tools":[]}], "B-team")
    run.validate_init([{"type":"system","subtype":"init","mcp_servers":[{"server":"agentboard"}],"tools":[]}], "B-team")
    for arm in run.MODELS:
        bad = [{"type":"system","subtype":"init","mcp_servers":["unexpected"],"tools":[]}]
        try: run.validate_init(bad, arm)
        except ValueError as e: check("INVALID" in str(e), "non-allowlisted MCP rejected")
        else: check(False, "non-allowlisted MCP accepted")


def test_e2e():
    task = {"id":"toy", "grade_task": str(HERE / "fixtures/toy/grade.json")}
    with tempfile.TemporaryDirectory() as td:
        scratch = Path(td)
        fake = str(HERE / "fake_claude.py")
        os.environ["BENCH_AGENTBOARD"] = str(ROOT / "target/release/agentboard")
        rows = []
        for arm in run.MODELS:
            os.environ["BENCH_TEST_ARM"] = arm
            os.environ["BENCH_TEST_SCENARIO"] = "normal"
            row = run.run_one(task, arm, fake, scratch, 1)
            rows.append(row)
            check(row["outcome"] == "done", f"{arm} completes")
            check(row["usage"]["sessions"] == (3 if arm == "B-team" else 1), f"{arm} session count")
            check(row["manager_tokens_excluded"] is True, "manager exclusion")
            check(row["grade"] == "accept=pass, regress=pass" and row["pass"] is True,
                  f"{arm} runs isolated grade.py acceptance and regression")
            d = json.loads((scratch / row["run_id"] / "card.json").read_text())
            if arm == "B-team":
                port = int(d["mcpServers"]["agentboard"]["url"].split(":")[2].split("/")[0])
                check(port > 0, "B-team scratch MCP card")
            else:
                check(d == {"mcpServers": {}}, "solo strict empty MCP config")
                port = int(env_rows_port := json.loads((scratch / row["run_id"] / "session-env.jsonl").read_text().splitlines()[0])["BENCH_SCRATCH_PORT"])
            check((scratch / row["run_id"] / "board").is_dir(), "scratch board")
            env_rows = [json.loads(line) for line in (scratch / row["run_id"] / "session-env.jsonl").read_text().splitlines()]
            check(len(env_rows) == row["usage"]["sessions"], f"{arm} session env captured")
            check(all(Path(e["AGENTBOARD_DB"]).is_dir() and Path(e["BENCH_MCP_CONFIG"]).is_file()
                      and int(e["BENCH_SCRATCH_PORT"]) == port for e in env_rows),
                  f"{arm} sessions receive isolated board and scratch card port")
        out = report.render({"rows": rows})
        data = scratch / "results.json"; data.write_text(json.dumps({"rows": rows}))
        rendered = scratch / "rendered.md"
        cli = subprocess.run([sys.executable, str(ROOT / "bench/report.py"), "--from", str(data), "--output", str(rendered)],
                             text=True, capture_output=True)
        check(cli.returncode == 0, "report.py --from succeeds")
        check(out.encode() == rendered.read_bytes(), "report.py --from regenerates byte-identical Markdown")
        check("Manager tokens excluded" in out, "report calls out excluded manager")
        check("## Aggregate" in out and "## toy" in out, "report has per-task and aggregate tables")
        os.environ.pop("BENCH_TEST_ARM", None)
        os.environ.pop("BENCH_TEST_SCENARIO", None)
        os.environ.pop("BENCH_AGENTBOARD", None)
    with tempfile.TemporaryDirectory() as td:
        os.environ["BENCH_TEST_ARM"] = "B-team"
        os.environ["BENCH_AGENTBOARD"] = str(ROOT / "target/release/agentboard")
        os.environ["BENCH_TEST_BAD_MCP"] = "1"
        row = run.run_one({"id": "toy"}, "B-team", str(HERE / "fake_claude.py"), Path(td), 1)
        check(row["outcome"] == "INVALID", "driver marks non-allowlisted MCP init INVALID")
        os.environ.pop("BENCH_TEST_ARM", None)
        os.environ.pop("BENCH_TEST_BAD_MCP", None)
        os.environ.pop("BENCH_AGENTBOARD", None)


def test_outcomes_and_verdicts():
    with tempfile.TemporaryDirectory() as td:
        scratch = Path(td)
        fake = str(HERE / "fake_claude.py")
        for scenario, expected, retries, reopens in [
            ("split", "split", 0, 0), ("blocked", "blocked", 0, 0),
            ("stalled", "stalled", 1, 0), ("cap", "cap", 0, 4),
            ("reopen", "done", 0, 1),
        ]:
            os.environ["BENCH_TEST_ARM"] = "B-team"
            os.environ["BENCH_TEST_SCENARIO"] = scenario
            row = run.run_one({"id": "toy"}, "B-team", fake, scratch, 1)
            check(row["outcome"] == expected, f"{scenario} driver outcome")
            check(row["usage"]["retries"] == retries, f"{scenario} retry count")
            check(row["usage"]["reopen_count"] == reopens, f"{scenario} reopen count")
        os.environ.pop("BENCH_TEST_ARM", None)
        os.environ.pop("BENCH_TEST_SCENARIO", None)
    cases = [(-1, 1, "worse"), (0, 3, "posture FAIL"), (0, 1.5, "tied, slight premium"),
             (0, 2, "tied, premium"), (1, 3, "better"), (1, 3.1, "better but expensive"),
             (None, None, "null result")]
    for delta, multiple, expected in cases:
        got = report.verdict(delta, multiple)
        check(got.startswith(expected), f"verdict {delta},{multiple}: {got}")
    partial = report.render({"run_status": "partial", "run_note": "Stopped after 1/12 cells.",
                             "rows": [{"task": "t1", "arm": "A-sonnet", "pass": True,
                                       "grade": "accept=pass, regress=pass", "outcome": "done",
                                       "usage": {"sessions":1,"tool_calls":0,"input_tokens":1,
                                                 "output_tokens":2,"cache_creation_input_tokens":3,
                                                 "cache_read_input_tokens":4,"total_cost_usd":0.1,
                                                 "wall_clock_ms":5}}]})
    check("Status: partial." in partial and "Stopped after 1/12 cells." in partial,
          "partial report explains stopped matrix")
    check("B-team vs A-sonnet: incomplete" in partial and "B-team vs A-opus: incomplete" in partial,
          "partial report retains both directional verdict lines")
    check("ΔQ=" not in partial and "posture FAIL" not in partial,
          "partial report cannot publish comparative verdicts")
    empty_usage = {key: 0 for key in ("sessions", "tool_calls", "input_tokens", "output_tokens",
                                      "cache_creation_input_tokens", "cache_read_input_tokens",
                                      "total_cost_usd", "wall_clock_ms")}
    contaminated = report.render({"rows": [{"task": "t1", "arm": "A-sonnet", "pass": None,
                                               "outcome": "INVALID", "usage": empty_usage}] * 12})
    check("incomplete (0/12 scoreable cells" in contaminated,
          "complete but invalid matrix cannot publish comparative verdicts")
    partial_scores = report.render({"rows": [
        {"task": f"t{i}", "arm": arm, "pass": result, "outcome": "done" if result is not None else "stalled",
         "usage": empty_usage}
        for i, arm, result in [(1, "A-sonnet", True), (1, "A-opus", False), (1, "B-team", None)]
    ]})
    check("incomplete (2/12 scoreable cells" in partial_scores,
          "incomplete report counts only rows with a pass result")
    usage = {"sessions":1,"tool_calls":0,"input_tokens":0,"output_tokens":0,
             "cache_creation_input_tokens":0,"cache_read_input_tokens":0,
             "total_cost_usd":0,"wall_clock_ms":0}
    values = report.render({"rows": [
        {"task":"values","arm":"A-sonnet","pass":True,"grade":"accept=pass, regress=pass","outcome":"done","usage":usage},
        {"task":"values","arm":"A-opus","pass":False,"grade":"boom | x","outcome":"grade-failed","usage":usage},
        {"task":"values","arm":"B-team","pass":None,"grade":None,"outcome":"stalled","usage":usage},
    ]})
    check("| A-sonnet | true | accept=pass, regress=pass |" in values,
          "PASS true stays a JSON boolean and grade has its own column")
    check("| A-opus | false | boom \\| x |" in values,
          "PASS false stays unchanged and Markdown pipes in grade are escaped")
    check("| B-team | null | — |" in values,
          "PASS null stays unchanged and missing grade renders as a dash")
    check("| A-opus | false | boom \\| x |" in values and "| A-opus | boom" not in values,
          "false PASS cell never contains grader text")


def main():
    test_usage_and_validation(); test_e2e(); test_outcomes_and_verdicts()
    print(f"bench: {checks}/{checks} passed")


if __name__ == "__main__": main()
