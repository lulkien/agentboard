"""Scratch-board proof for scripts/migrate-board.py. Run through tests/migrate.sh."""

import fcntl
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import tempfile

import lmdb


ROOT = pathlib.Path(__file__).resolve().parent.parent
SCRIPT = ROOT / "scripts/migrate-board.py"
BINARY = pathlib.Path(os.environ.get("AGENTBOARD_TEST_BIN", ROOT / "target/release/agentboard"))
spec = importlib.util.spec_from_file_location("migrate_board", SCRIPT)
migration = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migration)
checks = 0


def check(condition, message):
    global checks
    assert condition, message
    checks += 1


def run(*args):
    return subprocess.run(
        args, text=True, capture_output=True, timeout=10,
        env={**os.environ, "AGENTBOARD_MAP_SIZE_MB": "8"}
    )


def command(db, *args):
    return run(sys.executable, str(SCRIPT), "--db", str(db), *args)


def json_bytes(value):
    return json.dumps(value, separators=(",", ":")).encode()


def fixture(db, paths=("lean-display-server", "/abs/path/lean-display-server")):
    env = lmdb.open(str(db), map_size=8 * 1024 * 1024, max_dbs=8)
    with env.begin(write=True) as txn:
        tables = {name: env.open_db(name.encode(), txn=txn, create=True) for name in (
            "meta", "tasks", "events", "events_by_task", "agents",
            "tasks_by_status", "tasks_by_assignee"
        )}
        metadata = dict(application="agentboard-lmdb", format_version=1,
                        instance_id="fixture-instance", next_task_id=len(paths) + 1,
                        next_event_seq=len(paths) + 1, map_size_mb=8)
        txn.put(b"envelope", json_bytes(metadata), db=tables["meta"])
        for task_id, path in enumerate(paths, 1):
            key = task_id.to_bytes(8, "big")
            task = dict(id=task_id, title=f"task {task_id}", description="", goal="",
                        analyst_report="", coder_report="", reviewer_report="",
                        status="todo", priority=100, reporter="manager", assignee=None,
                        parent_id=None, path=path, revision=1, created_at_ms=1,
                        updated_at_ms=1)
            txn.put(key, json_bytes(task), db=tables["tasks"])
            index = b"\0" + (100 ^ 0x80000000).to_bytes(4, "big") + key
            txn.put(index, b"", db=tables["tasks_by_status"])
            event = dict(seq=task_id, task_id=task_id, actor="manager", kind="created",
                         timestamp_ms=1, recipient=None, payload={})
            txn.put(key, json_bytes(event), db=tables["events"])
            txn.put(key + key, b"", db=tables["events_by_task"])
        agent = dict(name="manager", first_seen_ms=1, last_seen_ms=1, last_task_id=len(paths))
        txn.put(b"manager", json_bytes(agent), db=tables["agents"])
    env.close()
    (db / "lifetime.lock").touch()


def snapshot(db):
    return (db / "data.mdb").read_bytes()


def test_main():
    source = (ROOT / "src/store/open.rs").read_text()
    binary_format = int(re.search(r"const FORMAT_VERSION: u32 = (\d+);", source).group(1))
    check(binary_format == migration.FORMAT_VERSION, "script format differs from binary")
    with tempfile.TemporaryDirectory(prefix="agentboard-migrate-") as temp:
        db = pathlib.Path(temp) / "board"
        db.mkdir()
        fixture(db)
        cursor = "fixture-instance:1"
        before = snapshot(db)
        for verb in ("ls", "serve"):
            result = run(str(BINARY), "--db", str(db), verb)
            check(result.returncode != 0, f"v1 {verb} opened")
            check("scripts/migrate-board.py" in result.stderr and str(db) in result.stderr,
                  f"v1 {verb} lacked migration remedy: {result.stderr}")
        args = ("--map", "lean-display-server=1", "--map", "/abs/path/lean-display-server=1")
        result = command(db, *args, "--dry-run")
        check(result.returncode == 0 and "dry run" in result.stdout, result.stderr)
        check(snapshot(db) == before, "dry run changed data.mdb")
        result = command(db, "--map", "lean-display-server=1")
        check(result.returncode == 1 and "unmapped path" in result.stderr, result.stderr)
        check(snapshot(db) == before, "unmapped path changed data.mdb")
        with (db / "lifetime.lock").open("rb") as held:
            fcntl.flock(held, fcntl.LOCK_SH)
            result = command(db, *args)
            check(result.returncode == 1 and "board is busy" in result.stderr, result.stderr)
            check(snapshot(db) == before, "busy refusal changed data.mdb")
        result = command(db, *args)
        check(result.returncode == 0 and "migrated 1->2" in result.stdout, result.stderr)
        for task_id in (1, 2):
            result = run(str(BINARY), "--db", str(db), "get", str(task_id))
            check(result.returncode == 0 and "project 1" in result.stdout, result.stderr)
        result = run(str(BINARY), "--db", str(db), "feed", "--after", cursor)
        check(result.returncode == 0 and "created" in result.stdout and "T2" in result.stdout,
              f"old cursor did not replay event 2: {result.stderr} {result.stdout}")
        check("fixture-instance:" in result.stdout, result.stdout)
        result = run(str(BINARY), "--db", str(db), "agents")
        check(result.returncode == 0 and "manager" in result.stdout, result.stderr)
        result = run(str(BINARY), "--db", str(db), "ls")
        check(result.returncode == 0, result.stderr)
        result = run(str(BINARY), "--db", str(db), "serve")
        check("mcp server: connection closed: initialize request" in result.stderr,
              f"serve did not reach MCP startup: {result.stderr}")
        result = command(db, *args)
        check(result.returncode == 0 and "up to date" in result.stdout, result.stderr)
        env = lmdb.open(str(db), max_dbs=8, create=False)
        with env.begin() as txn:
            meta = env.open_db(b"meta", txn=txn, create=False)
            projects = env.open_db(b"projects", txn=txn, create=False)
            metadata = migration.read_metadata(txn, meta)
            check(metadata["instance_id"] == "fixture-instance" and metadata["next_event_seq"] == 3,
                  "cursors or event sequence changed")
            check(metadata["next_task_id"] == 3 and metadata["next_project_id"] == 2,
                  "task/project counters changed incorrectly")
            project = migration.decode(txn.get((1).to_bytes(8, "big"), db=projects))
            check(project["project_path"] == "/abs/path/lean-display-server", project)
            events = env.open_db(b"events", txn=txn, create=False)
            agents = env.open_db(b"agents", txn=txn, create=False)
            check(migration.decode(txn.get((2).to_bytes(8, "big"), db=events))["task_id"] == 2,
                  "event bytes were not preserved")
            check(migration.decode(txn.get(b"manager", db=agents))["last_task_id"] == 2,
                  "agent roster was not preserved")
        env.close()

        with tempfile.TemporaryDirectory(prefix="agentboard-migrate-newer-") as newer_temp:
            newer = pathlib.Path(newer_temp)
            fixture(newer, (None,))
            newer_env = lmdb.open(str(newer), max_dbs=8, map_size=8 * 1024 * 1024)
            with newer_env.begin(write=True) as txn:
                newer_meta = newer_env.open_db(b"meta", txn=txn, create=False)
                metadata = migration.read_metadata(txn, newer_meta)
                metadata["format_version"] = 3
                txn.put(b"envelope", migration.encode(metadata), db=newer_meta)
            newer_env.close()
            result = command(newer)
            check(result.returncode == 1 and "newer than this script" in result.stderr,
                  result.stderr)
            result = run(str(BINARY), "--db", str(newer), "ls")
            check(result.returncode != 0 and "binary is older than the board" in result.stderr,
                  result.stderr)


def test_step_chain():
    with tempfile.TemporaryDirectory(prefix="agentboard-migrate-chain-") as temp:
        db = pathlib.Path(temp)
        fixture(db, (None,))
        env = lmdb.open(str(db), max_dbs=8, create=False)
        seen = []
        original_target = migration.FORMAT_VERSION
        original_step = migration.STEPS.get(2)

        def fake_v2_to_v3(env, mappings):
            with env.begin(write=True) as txn:
                meta = env.open_db(b"meta", txn=txn, create=False)
                metadata = migration.read_metadata(txn, meta)
                seen.append(metadata["format_version"])
                metadata["format_version"] = 3
                txn.put(b"envelope", migration.encode(metadata), db=meta)

        try:
            migration.FORMAT_VERSION = 3
            migration.STEPS[2] = fake_v2_to_v3
            result = migration.migrate(env, {})
            check("1->2" in result and "2->3" in result, result)
            check(seen == [2], f"step order was {seen}")
            with env.begin() as txn:
                meta = env.open_db(b"meta", txn=txn, create=False)
                check(migration.read_metadata(txn, meta)["format_version"] == 3,
                      "fake step did not stamp 3")
        finally:
            migration.FORMAT_VERSION = original_target
            if original_step is None:
                migration.STEPS.pop(2)
            else:
                migration.STEPS[2] = original_step
            env.close()

    with tempfile.TemporaryDirectory(prefix="agentboard-migrate-retry-") as temp:
        db = pathlib.Path(temp)
        fixture(db, (None,))
        env = lmdb.open(str(db), max_dbs=8, map_size=8 * 1024 * 1024)
        original_target = migration.FORMAT_VERSION
        original_step = migration.STEPS.get(2)

        def fail_v2_to_v3(env, mappings):
            raise migration.MigrationError("injected step failure")

        try:
            migration.FORMAT_VERSION = 3
            migration.STEPS[2] = fail_v2_to_v3
            try:
                migration.migrate(env, {})
            except migration.MigrationError as error:
                check("injected step failure" in str(error), str(error))
            else:
                raise AssertionError("fake failure did not stop migration")
            with env.begin() as txn:
                meta = env.open_db(b"meta", txn=txn, create=False)
                check(migration.read_metadata(txn, meta)["format_version"] == 2,
                      "completed 1->2 step did not survive failed 2->3")
        finally:
            migration.FORMAT_VERSION = original_target
            if original_step is None:
                migration.STEPS.pop(2)
            else:
                migration.STEPS[2] = original_step
            env.close()


if __name__ == "__main__":
    test_main()
    test_step_chain()
    print(f"migrate: {checks}/{checks} passed (scratch LMDB, stepwise)")
