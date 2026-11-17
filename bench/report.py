#!/usr/bin/env python3
"""Render benchmark JSON deterministically."""
import argparse
import json
from pathlib import Path


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
    lines = ["# Benchmark results", "", "Manager tokens excluded; B-team cost is a lower bound.", "",
             "| Task | Arm | PASS | Outcome | Sessions | Tool calls | Input | Output | Cache write | Cache read | USD | Wall ms |",
             "| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for r in rows:
        u = r["usage"]
        lines.append(f"| {r['task']} | {r['arm']} | {r['pass']} | {r['outcome']} | {u['sessions']} | {u['tool_calls']} | {u['input_tokens']} | {u['output_tokens']} | {u['cache_creation_input_tokens']} | {u['cache_read_input_tokens']} | {u['total_cost_usd']:.6f} | {u['wall_clock_ms']} |")
    lines += ["", "## Verdicts", "", "Verdict thresholds are proposals, not a product SLA.", ""]
    for base in ("A-sonnet", "A-opus"):
        b = [r for r in rows if r["arm"] == "B-team"]
        a = [r for r in rows if r["arm"] == base]
        if not b or not a: continue
        bp = sum(r["pass"] is True for r in b); ap = sum(r["pass"] is True for r in a)
        bu = sum(r["usage"]["total_cost_usd"] for r in b); au = sum(r["usage"]["total_cost_usd"] for r in a)
        multiple = bu / au if au else (None if bu == 0 else float("inf"))
        label = "null" if multiple is None else "infinite" if multiple == float("inf") else f"{multiple:.2f}x"
        result = "undefined (zero baseline cost)" if multiple is None else verdict(bp-ap, multiple)
        lines.append(f"B-team vs {base}: ΔQ={bp-ap}, M={label}, {result}.")
    lines.append("")
    return "\n".join(lines)


def main():
    p = argparse.ArgumentParser(); p.add_argument("--from", dest="source", required=True, type=Path); p.add_argument("--output", type=Path)
    a = p.parse_args(); text = render(json.loads(a.source.read_text()))
    if a.output: a.output.write_text(text)
    else: print(text, end="")


if __name__ == "__main__": main()
