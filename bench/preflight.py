#!/usr/bin/env python3
"""Validate the frozen historical corpus with offline builds and hidden graders."""

import json
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "bench" / "corpus"
SCRATCH = Path("/tmp/agentboard/scratch/bench-preflight")
EXPECTED_IDS = {
    "t1-actor-guard", "t2-claim-doc", "t3-wait-after", "t4-map-usage"
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--task", help="check only this corpus task id")
    args = parser.parse_args()
    SCRATCH.mkdir(parents=True, exist_ok=True)
    tasks = sorted(CORPUS.glob("*/task.json"))
    ids = {json.loads(path.read_text())["id"] for path in tasks}
    if not args.task and ids != EXPECTED_IDS:
        raise RuntimeError(f"expected frozen task ids {sorted(EXPECTED_IDS)}, found {sorted(ids)}")
    if args.task:
        tasks = [path for path in tasks if json.loads(path.read_text())["id"] == args.task]
    if not tasks:
        raise RuntimeError("no task.json files found")
    passed = 0
    for task_path in tasks:
        task = json.loads(task_path.read_text())
        for variant in ("base", "ref"):
            result = subprocess.run(
                [sys.executable, str(ROOT / "bench" / "grade.py"), str(task_path),
                 variant, "--scratch", str(SCRATCH)],
                cwd=ROOT, text=True, stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT)
            if result.returncode:
                print(f"{task['id']}: FAIL ({variant})")
                print(result.stdout, end="")
                return 1
        print(f"{task['id']}: valid (fail-to-pass, pass-to-pass)")
        passed += 1
    print(f"preflight: {passed}/{len(tasks)} tasks valid (fail-to-pass, pass-to-pass)")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, ValueError, RuntimeError) as exc:
        print(f"preflight: {exc}", file=sys.stderr)
        raise SystemExit(1)
