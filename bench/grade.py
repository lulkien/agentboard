#!/usr/bin/env python3
"""Build and grade one archived benchmark tree using only the standard library."""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]


def run(command, cwd, env=None, check=True):
    return subprocess.run(command, cwd=cwd, env=env, text=True,
                          stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          check=check)


def archive(commit, destination):
    destination.mkdir(parents=True, exist_ok=True)
    data = subprocess.run(["git", "-C", str(ROOT), "archive", commit],
                          stdout=subprocess.PIPE, check=True).stdout
    import io
    import tarfile
    with tarfile.open(fileobj=io.BytesIO(data)) as tar:
        tar.extractall(destination, filter="data")


def fixture_tree(task, commit, destination):
    base = task["base"]
    shutil.rmtree(destination, ignore_errors=True)
    shutil.rmtree(destination.parent / (destination.name + "-candidate"), ignore_errors=True)
    archive(base, destination)
    run(["git", "init", "-q"], destination)
    run(["git", "config", "user.name", "Agentboard preflight"], destination)
    run(["git", "config", "user.email", "preflight@invalid"], destination)
    run(["git", "add", "-A"], destination)
    run(["git", "commit", "-qm", "base"], destination)
    if commit != base:
        candidate = destination.parent / (destination.name + "-candidate")
        archive(commit, candidate)
        shutil.copytree(candidate, destination, dirs_exist_ok=True)
        shutil.rmtree(candidate)
    # Git archives preserve commit timestamps. Touch the source snapshot so Cargo
    # cannot reuse a same-second fingerprint from another historical revision.
    for path in destination.rglob("*"):
        if path.is_file() and ".git" not in path.parts:
            os.utime(path, None)
    if commit != base:
        run(["git", "add", "-A"], destination)
        run(["git", "commit", "-qm", "candidate"], destination)
    return destination


def grade_tree(task_path, tree, scratch, expect_fail=False):
    task = json.loads(task_path.read_text())
    task_dir = task_path.parent
    tree = tree.resolve()
    if not tree.is_dir():
        raise RuntimeError(f"tree does not exist: {tree}")
    target = scratch / "cargo-target"
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(target)
    build = run(task["build"], tree, env=env, check=False)
    if build.returncode:
        raise RuntimeError(f"build failed ({build.returncode}):\n{build.stdout[-4000:]}")
    binary = scratch / f"agentboard-{task['id']}-{tree.name}"
    shutil.copy2(target / "release" / "agentboard", binary)
    accept = run(["bash", str(task_dir / task["accept"]), str(binary), str(tree)],
                 tree, env=env, check=False)
    expected = not expect_fail
    if (accept.returncode == 0) != expected:
        expectation = "pass" if expected else "fail"
        raise RuntimeError(f"accept expected to {expectation}, returned {accept.returncode}:\n{accept.stdout[-2500:]}")
    for command in task["regress"]:
        regress_env = env.copy()
        regress_env["AGENTBOARD_TEST_BIN"] = str(binary)
        result = run(command, tree, env=regress_env, check=False)
        if result.returncode:
            raise RuntimeError(f"regress {command!r} failed:\n{result.stdout[-4000:]}")
    return "accept=fail" if not expected else "accept=pass, regress=pass"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("task", type=Path)
    parser.add_argument("tree", help="tree path, or the base/ref corpus fixture")
    parser.add_argument("--expect-fail", action="store_true",
                        help="require hidden acceptance to fail (used for base preflight)")
    parser.add_argument("--scratch", type=Path, required=True)
    args = parser.parse_args()
    task_path = args.task.resolve()
    task = json.loads(task_path.read_text())
    if args.tree in ("base", "ref"):
        tree = fixture_tree(task, task[args.tree], args.scratch.resolve() /
                            f"{task['id']}-{args.tree}")
        expect_fail = args.tree == "base"
    else:
        tree = Path(args.tree)
        expect_fail = args.expect_fail
    print(grade_tree(task_path, tree, args.scratch.resolve(), expect_fail))


if __name__ == "__main__":
    try:
        main()
    except (OSError, subprocess.CalledProcessError, RuntimeError, ValueError) as exc:
        print(f"grade: {exc}", file=sys.stderr)
        raise SystemExit(1)
