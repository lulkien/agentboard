#!/usr/bin/env python3
"""Offline-first driver for the solo and agentboard benchmark arms."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
MODELS = {
    "A-sonnet": [("solo", "claude-sonnet-5-5", "medium")],
    "A-opus": [("solo", "claude-opus-5-5", "high")],
    "B-team": [("analyst", "claude-opus-5-5", "high"),
               ("coder", "claude-sonnet-5-5", "medium"),
               ("reviewer", "claude-opus-5-5", "high")],
}
ALLOWED_MCP = {"A-sonnet": set(), "A-opus": set(), "B-team": {"agentboard"}}


def plan():
    print("arms: A-sonnet, A-opus, B-team")
    print("sessions: analyst=opus/high coder=sonnet/medium reviewer=opus/high")
    print("caps: $6/session $20/task-arm; B manager tokens excluded (0 deterministic tokens)")
    print("corpus: " + ", ".join(p.name for p in sorted((ROOT / "bench/corpus").iterdir()) if p.is_dir()))


def usage(events):
    result = {"input_tokens": 0, "output_tokens": 0,
              "cache_creation_input_tokens": 0, "cache_read_input_tokens": 0,
              "total_cost_usd": 0.0, "duration_ms": 0, "num_turns": 0,
              "tool_calls": 0}
    for event in events:
        if event.get("type") == "assistant":
            result["tool_calls"] += sum(b.get("type") == "tool_use" for b in event.get("message", {}).get("content", []))
        if event.get("type") == "result":
            u = event.get("usage", {})
            for k in ("input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "duration_ms", "num_turns"):
                result[k] += int(u.get(k, 0) or 0)
            result["total_cost_usd"] += float(event.get("total_cost_usd", 0) or 0)
    return result


def validate_init(events, arm):
    init = next((e for e in events if e.get("type") == "system" and e.get("subtype") == "init"), None)
    if init is None:
        raise ValueError("missing system/init event")
    servers = set(init.get("mcp_servers", []))
    tools = init.get("tools", [])
    names = {t.get("server") for t in tools if isinstance(t, dict) and t.get("server")}
    names |= {s.get("name", "").split("__", 2)[1] for s in tools
              if isinstance(s, dict) and s.get("name", "").startswith("mcp__")}
    allowed = ALLOWED_MCP[arm]
    if not servers <= allowed or not names <= allowed:
        raise ValueError(f"INVALID: non-allowed MCP servers/tools: {sorted((servers | names) - allowed)}")


def session(command, model, effort, prompt, env, arm):
    argv = [command, "-p", prompt, "--model", model, "--effort", effort,
            "--output-format", "stream-json", "--verbose", "--dangerously-skip-permissions",
            "--max-budget-usd", "6"]
    if arm == "B-team":
        argv += ["--safe-mode", "--mcp-config", env["BENCH_MCP_CONFIG"], "--strict-mcp-config"]
    p = subprocess.run(argv, env=env, cwd=env.get("BENCH_TREE"), text=True,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if p.returncode:
        raise RuntimeError(f"claude exited {p.returncode}: {p.stdout[-2000:]}")
    events = [json.loads(line) for line in p.stdout.splitlines() if line.strip()]
    validate_init(events, arm)
    return events, usage(events)


def board_row(env, row_id):
    result = subprocess.run([env.get("BENCH_AGENTBOARD", "agentboard"), "get", str(row_id)],
                            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    if result.returncode:
        raise RuntimeError(f"scratch board get failed: {result.stdout[-1000:]}")
    head = result.stdout.splitlines()[0].split()
    return head[1], (None if head[4] == "-" else head[4])


def corpus_tasks():
    return [json.loads(p.read_text()) for p in sorted((ROOT / "bench/corpus").glob("*/task.json"))]


def run_one(task, arm, command, scratch, reps):
    prompt_path = ROOT / "bench/corpus" / task["id"] / "prompt.md"
    prompt = prompt_path.read_text() if prompt_path.exists() else task.get("prompt", "")
    totals, sessions, retries, reopen_count = usage([]), [], 0, 0
    run_dir = scratch / uuid.uuid4().hex
    run_dir.mkdir(parents=True)
    db = run_dir / "board"
    db.mkdir()
    port = 41000 + (os.getpid() % 20000)
    card = run_dir / "card.json"
    card.write_text(json.dumps({"mcpServers": {"agentboard": {"url": f"http://127.0.0.1:{port}/mcp/bench"}}}))
    env = os.environ.copy()
    env.update(AGENTBOARD_DB=str(db), BENCH_MCP_CONFIG=str(card), BENCH_SCRATCH_PORT=str(port))
    env["AGENTBOARD_ACTOR"] = "bench"
    env["BENCH_ROW_ID"] = ""
    created = subprocess.run([env.get("BENCH_AGENTBOARD", "agentboard"), "add", "benchmark task",
                              "--project", "0", "--status", "planning", "--assignee", "analyst",
                              "--new-actor"], env=env, text=True, stdout=subprocess.PIPE,
                             stderr=subprocess.STDOUT)
    if created.returncode:
        raise RuntimeError(f"scratch board task creation failed: {created.stdout[-1000:]}")
    row_id = int(next(line.split()[0][1:] for line in created.stdout.splitlines() if line.startswith("T")))
    env["BENCH_ROW_ID"] = str(row_id)
    corpus_task = Path(task.get("grade_task", ROOT / "bench/corpus" / task["id"] / "task.json"))
    tree = run_dir / "tree"
    if corpus_task.is_file() and "grade_task" not in task:
        # Export only the declared historical base; the session cannot see the
        # working checkout or reference commit through its working directory.
        prep = subprocess.run([sys.executable, str(ROOT / "bench/grade.py"),
                               str(corpus_task), "base", "--scratch", str(run_dir / "grade")],
                              text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        if prep.returncode:
            raise RuntimeError(f"isolated base preparation failed: {prep.stdout[-2000:]}")
        tree = run_dir / "grade" / f"{task['id']}-base"
        env["BENCH_TREE"] = str(tree)
    else:
        tree.mkdir()
    started = time.monotonic()
    roles = MODELS[arm]
    outcome = "done"
    stage = 0
    last_status, last_assignee = "todo", None
    stage_limit = 10 if arm == "B-team" else 1
    while stage < stage_limit:
        seat, model, effort = roles[min(stage, len(roles)-1)]
        seat_prompt = prompt if arm != "B-team" else f"Read {ROOT / 'skills' / f'agentboard-{seat}' / 'SKILL.md'}; task T1.\n" + prompt
        try:
            events, counts = session(command, model, effort, seat_prompt, env, arm)
        except ValueError as exc:
            outcome = "INVALID" if "INVALID" in str(exc) else "error"
            break
        sessions.append({"seat": seat, **counts})
        for key in totals:
            totals[key] += counts[key]
        if arm != "B-team":
            break
        status, assignee = board_row(env, row_id)
        if status == "todo":
            outcome = "split"; break
        elif status == "blocked":
            outcome = "blocked"; break
        elif status == last_status and assignee == last_assignee:
            if retries == 0:
                retries += 1
                continue  # deterministic manager retries this seat once
            outcome = "stalled"; break
        elif last_status == "reviewing" and status == "implementing":
            reopen_count += 1
            stage = 1  # reviewer returned the row to coder
        elif status == "done":
            break
        else:
            stage += 1
        last_status, last_assignee = status, assignee
        if len(sessions) >= stage_limit and status != "done":
            outcome = "cap"; break
    totals["wall_clock_ms"] = int((time.monotonic() - started) * 1000)
    totals["sessions"] = len(sessions)
    totals["retries"] = retries
    totals["reopen_count"] = reopen_count
    grade_result = None
    if corpus_task.is_file() and outcome == "done":
        graded = subprocess.run([sys.executable, str(ROOT / "bench/grade.py"),
                                 str(corpus_task), str(tree), "--scratch", str(run_dir / "grade-final")],
                                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        grade_result = graded.stdout.strip().splitlines()[-1] if graded.stdout.strip() else ""
        outcome = "done" if graded.returncode == 0 else "grade-failed"
    passed = True if grade_result == "accept=pass, regress=pass" else False if grade_result else None
    return {"task": task["id"], "arm": arm, "outcome": outcome,
            "pass": passed, "grade": grade_result, "usage": totals, "seat_usage": sessions,
            "manager_tokens_excluded": True, "run_id": run_dir.name}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--plan", action="store_true")
    p.add_argument("--task", action="append")
    p.add_argument("--arm", action="append", choices=MODELS)
    p.add_argument("--reps", type=int, default=1)
    p.add_argument("--output", type=Path, default=Path("bench-results.json"))
    args = p.parse_args()
    if args.plan:
        plan(); return
    command = os.environ.get("BENCH_CLAUDE", "claude")
    selected = set(args.task or [])
    tasks = [t for t in corpus_tasks() if not selected or t["id"] in selected]
    arms = args.arm or list(MODELS)
    scratch_root = Path("/tmp/agentboard/scratch")
    scratch_root.mkdir(parents=True, exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="run-", dir=scratch_root))
    rows = []
    try:
        for _ in range(args.reps):
            for task in tasks:
                for arm in arms:
                    rows.append(run_one(task, arm, command, scratch, args.reps))
        args.output.write_text(json.dumps({"rows": rows, "manager_tokens_excluded": True}, indent=2) + "\n")
        print(f"benchmark: {len(rows)} cells -> {args.output}")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


if __name__ == "__main__":
    try: main()
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as exc:
        print(f"bench: {exc}", file=sys.stderr); raise SystemExit(1)
