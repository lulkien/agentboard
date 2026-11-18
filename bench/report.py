#!/usr/bin/env python3
"""Render benchmark JSON deterministically."""
import argparse
import json
from pathlib import Path

EXPECTED_CELLS = 12


def verdict(delta, multiple):
    if delta is None or multiple is None: return "null result"
    if delta < 0: return "worse"
    if delta <= 0 and multiple >= 3: return "posture FAIL"
    if delta == 0 and multiple <= 1.5: return "tied, slight premium"
    if delta == 0: return f"tied, premium {multiple:.2f} (owner judges)"
    if multiple <= 3: return "better"
    return "better but expensive"


def render(data):
    rows = data["rows"]
    tasks = {row["task"] for row in rows}
    arms = ("A-sonnet", "A-opus", "B-team")
    scoreable_cells = sum(isinstance(row.get("pass"), bool) for row in rows)
    pairs = [(row["task"], row["arm"]) for row in rows]
    scoreable = (len(rows) == EXPECTED_CELLS and len(tasks) == 4
                 and len(set(pairs)) == EXPECTED_CELLS
                 and all(pairs.count((task, arm)) == 1 for task in tasks for arm in arms)
                 and scoreable_cells == EXPECTED_CELLS)
    lines = ["# Benchmark results", ""]
    if data.get("harness_commit"):
        lines.append(f"Harness commit: `{data['harness_commit']}`.")
    if data.get("claude_version"):
        lines.append(f"Claude: {data['claude_version']}.")
    if data.get("models"):
        models = "; ".join(f"{arm}: {', '.join(f'{seat}={model}/{effort}' for seat, model, effort in seats)}"
                            for arm, seats in data["models"].items())
        lines.append(f"Models and effort: {models}.")
    if data.get("repetitions") is not None:
        lines.append(f"Repetitions: {data['repetitions']}; caps: $6/session, $20/task-arm.")
    if data.get("isolation"):
        lines.append(f"Isolation: {data['isolation']}.")
    if data.get("run_status"):
        lines.append(f"Status: {data['run_status']}.")
    if data.get("run_note"):
        lines.append(data["run_note"])
    lines += ["", "Manager tokens excluded; B-team cost is a lower bound.", ""]

    def table(title, selected, aggregate=False):
        lines.extend([f"## {title}", "",
                      "| Arm | PASS | Grade | Outcome | Sessions | Tool calls | Input | Output | Cache write | Cache read | USD | Wall ms | B diagnostics |",
                      "| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |"])
        by_arm = {}
        for row in selected:
            by_arm.setdefault(row["arm"], []).append(row)
        for arm, group in by_arm.items():
            if aggregate:
                u = {key: sum(r["usage"].get(key, 0) for r in group)
                     for key in ("sessions", "tool_calls", "input_tokens", "output_tokens",
                                 "cache_creation_input_tokens", "cache_read_input_tokens",
                                 "total_cost_usd", "wall_clock_ms", "retries", "reopen_count")}
                passed = sum(r["pass"] is True for r in group)
                pass_text = f"{passed}/{len(group)}"
                grade_text = "—"
                outcome = ", ".join(sorted({r["outcome"] for r in group}))
            else:
                r = group[0]
                u = r["usage"]
                pass_text = json.dumps(r["pass"])
                grade_text = (r.get("grade") or "—").replace("|", "\\|")
                outcome = r["outcome"]
            diagnostic = f"retries={u.get('retries', 0)}, reopens={u.get('reopen_count', 0)}" if arm == "B-team" else "—"
            lines.append(f"| {arm} | {pass_text} | {grade_text} | {outcome} | {u['sessions']} | {u['tool_calls']} | {u['input_tokens']} | {u['output_tokens']} | {u['cache_creation_input_tokens']} | {u['cache_read_input_tokens']} | {u['total_cost_usd']:.6f} | {u['wall_clock_ms']} | {diagnostic} |")
        lines.append("")

    for task in dict.fromkeys(r["task"] for r in rows):
        table(task, [r for r in rows if r["task"] == task])
    table("Aggregate", rows, aggregate=True)
    lines += ["## Verdicts", "", "directional (n=4); verdict thresholds are proposals, not a product SLA.", ""]
    for base in ("A-sonnet", "A-opus"):
        b = [r for r in rows if r["arm"] == "B-team"]
        a = [r for r in rows if r["arm"] == base]
        if not scoreable or len(b) != 4 or len(a) != 4:
            lines.append(f"directional (n=4), B-team vs {base}: incomplete ({scoreable_cells}/12 scoreable cells; insufficient comparison data).")
            continue
        bp = sum(r["pass"] is True for r in b); ap = sum(r["pass"] is True for r in a)
        bu = sum(r["usage"]["total_cost_usd"] for r in b); au = sum(r["usage"]["total_cost_usd"] for r in a)
        multiple = bu / au if au else (None if bu == 0 else float("inf"))
        label = "null" if multiple is None else "infinite" if multiple == float("inf") else f"{multiple:.2f}x"
        result = "undefined (zero baseline cost)" if multiple is None else verdict(bp-ap, multiple)
        lines.append(f"directional (n=4), B-team vs {base}: ΔQ={bp-ap}, M={label}, {result}.")
    lines.append("")
    return "\n".join(lines)


def main():
    p = argparse.ArgumentParser(); p.add_argument("--from", dest="source", required=True, type=Path); p.add_argument("--output", type=Path)
    a = p.parse_args(); text = render(json.loads(a.source.read_text()))
    if a.output: a.output.write_text(text)
    else: print(text, end="")


if __name__ == "__main__": main()
