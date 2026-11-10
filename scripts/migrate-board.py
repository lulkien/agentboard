#!/usr/bin/env python3
"""Migrate an agentboard LMDB directory one format version at a time.

Run with: uv run --with lmdb python3 scripts/migrate-board.py --db DIR --map PATH=ID
The Python lmdb package is the only extra dependency. Keep a backup of the
board directory before an in-place migration.
"""

import argparse
import fcntl
import json
import pathlib
import sys

import lmdb


FORMAT_VERSION = 2  # Keep equal to src/store/open.rs FORMAT_VERSION.
APPLICATION = "agentboard-lmdb"
ENVELOPE_KEY = b"envelope"


class MigrationError(Exception):
    pass


def decode(raw):
    return json.loads(raw)


def encode(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False).encode()


def read_metadata(txn, meta):
    raw = txn.get(ENVELOPE_KEY, db=meta)
    if raw is None:
        raise MigrationError("missing metadata envelope")
    metadata = decode(raw)
    if metadata.get("application") != APPLICATION:
        raise MigrationError("directory is not an agentboard LMDB board")
    return metadata


def parse_maps(values):
    mappings = {}
    for value in values:
        path, separator, raw_id = value.rpartition("=")
        if not separator or not path or not raw_id.isdecimal() or int(raw_id) == 0:
            raise MigrationError(f"invalid --map {value!r}; expected nonempty PATH=positive ID")
        project_id = int(raw_id)
        if project_id > 2**64 - 2:
            raise MigrationError(f"project ID out of range: {project_id}")
        if path in mappings and mappings[path] != project_id:
            raise MigrationError(f"conflicting mappings for {path!r}")
        mappings[path] = project_id
    return mappings


def project_rows(mappings):
    """One project per ID; prefer an absolute alias as its registered path."""
    aliases = {}
    for path, project_id in mappings.items():
        aliases.setdefault(project_id, []).append(path)
    projects = {}
    for project_id, paths in aliases.items():
        project_path = next((p for p in paths if pathlib.Path(p).is_absolute()), paths[0])
        name = pathlib.Path(project_path).name
        if project_path.strip() != project_path or any(ord(c) < 32 for c in project_path):
            raise MigrationError(f"invalid project path {project_path!r}")
        if not name or name.strip() != name or any(ord(c) < 32 for c in name):
            raise MigrationError(f"invalid project name derived from {project_path!r}")
        projects[project_id] = {
            "project_id": project_id,
            "project_name": name,
            "project_path": project_path,
        }
    return projects


def planned_tasks(txn, tasks, mappings):
    changes = []
    with txn.cursor(db=tasks) as cursor:
        for key, raw in cursor:
            task = decode(raw)
            old_path = task.get("path")
            if old_path is not None and not isinstance(old_path, str):
                raise MigrationError(f"invalid path on task T{task.get('id')}")
            if old_path and old_path not in mappings:
                raise MigrationError(f"unmapped path {old_path!r} on task T{task.get('id')}")
            task.pop("path", None)
            task["project_id"] = mappings.get(old_path) if old_path else None
            changes.append((key, encode(task)))
    return changes


def v1_to_v2(env, mappings):
    """One atomic write: projects, converted tasks, then the version stamp."""
    projects = project_rows(mappings)
    with env.begin(write=True) as txn:
        meta = env.open_db(b"meta", txn=txn, create=False)
        tasks = env.open_db(b"tasks", txn=txn, create=False)
        metadata = read_metadata(txn, meta)
        if metadata["format_version"] != 1:
            raise MigrationError("v1_to_v2 expected format 1")
        changes = planned_tasks(txn, tasks, mappings)
        project_db = env.open_db(b"projects", txn=txn, create=True)
        for project_id, project in projects.items():
            key = project_id.to_bytes(8, "big")
            existing = txn.get(key, db=project_db)
            if existing is not None and decode(existing) != project:
                raise MigrationError(f"project {project_id} conflicts with mapping")
            txn.put(key, encode(project), db=project_db)
        for key, value in changes:
            txn.put(key, value, db=tasks)
        metadata["next_project_id"] = max(
            metadata.get("next_project_id", 1), max(projects, default=0) + 1
        )
        metadata["format_version"] = 2
        txn.put(ENVELOPE_KEY, encode(metadata), db=meta)
    return len(changes), len(projects)


STEPS = {1: v1_to_v2}


def migrate(env, mappings, dry_run=False):
    meta = env.open_db(b"meta", create=False)
    with env.begin() as txn:
        metadata = read_metadata(txn, meta)
        version = metadata["format_version"]
    if version > FORMAT_VERSION:
        raise MigrationError(
            f"board format {version} is newer than this script's {FORMAT_VERSION}; upgrade the script"
        )
    if version == FORMAT_VERSION:
        return "up to date"
    plan = []
    while version < FORMAT_VERSION:
        if version not in STEPS:
            raise MigrationError(f"no migration step from format {version}")
        plan.append(version)
        version += 1
    if dry_run:
        if 1 in plan:
            with env.begin() as txn:
                tasks = env.open_db(b"tasks", txn=txn, create=False)
                count = len(planned_tasks(txn, tasks, mappings))
            project_count = len(project_rows(mappings))
            return f"dry run: {count} tasks, {project_count} projects; steps " + " -> ".join(
                f"{v}->{v + 1}" for v in plan
            )
        return "dry run: steps " + " -> ".join(f"{v}->{v + 1}" for v in plan)
    results = []
    for version in plan:
        result = STEPS[version](env, mappings)
        with env.begin() as txn:
            stamped = read_metadata(txn, meta)["format_version"]
        if stamped != version + 1:
            raise MigrationError(f"step {version}->{version + 1} did not stamp its target version")
        results.append(f"{version}->{version + 1}" + (f" ({result[0]} tasks, {result[1]} projects)" if result else ""))
    return "migrated " + ", ".join(results)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True, type=pathlib.Path)
    parser.add_argument("--map", action="append", default=[], metavar="PATH=ID")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    try:
        mappings = parse_maps(args.map)
        path = args.db.resolve(strict=True)
        if not path.is_dir() or not (path / "data.mdb").is_file():
            raise MigrationError(f"not an LMDB board: {path}")
        lock_path = path / "lifetime.lock"
        if lock_path.is_symlink() or not lock_path.is_file():
            raise MigrationError(f"missing or unsafe lifetime lock: {lock_path}")
        with lock_path.open("rb") as lock:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError as error:
                raise MigrationError("board is busy; close attached binary processes") from error
            inspect = lmdb.open(str(path), max_dbs=8, create=False, readonly=True)
            try:
                meta = inspect.open_db(b"meta", create=False)
                with inspect.begin() as txn:
                    map_size_mb = read_metadata(txn, meta)["map_size_mb"]
            finally:
                inspect.close()
            env = lmdb.open(
                str(path), max_dbs=8, create=False, readonly=args.dry_run,
                map_size=map_size_mb * 1024 * 1024
            )
            try:
                print(migrate(env, mappings, args.dry_run))
            finally:
                env.close()
    except (MigrationError, lmdb.Error, OSError, ValueError) as error:
        print(f"migrate-board: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
