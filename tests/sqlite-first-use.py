#!/usr/bin/env python3
"""Eight simultaneous cold-process adds, exclusively on fresh scratch files."""
import concurrent.futures
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import threading

binary = Path(os.environ.get('AGENTBOARD_TEST_BIN', 'target/release/agentboard')).resolve()
env = dict(os.environ)
env.pop('AGENTBOARD_DB', None)
env.pop('AGENTBOARD_SURREAL_URL', None)
help_text = subprocess.check_output([binary, '--help'], text=True, env=env)
assert 'Data directory of the local board server' not in help_text, 'requires SQLite build'
with tempfile.TemporaryDirectory(prefix='agentboard-sqlite-first-use-') as scratch:
    for round_number in range(20):
        db = Path(scratch) / f'{round_number}.db'
        barrier = threading.Barrier(8)

        def add(worker):
            barrier.wait()
            result = subprocess.run(
                [binary, '--db', db, 'add', f'cold-{worker}'],
                env=env, capture_output=True, text=True, timeout=15,
            )
            assert result.returncode == 0, (round_number, worker, result.stdout, result.stderr)

        with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
            list(pool.map(add, range(8)))
        with sqlite3.connect(db) as connection:
            assert connection.execute('SELECT count(*) FROM tasks').fetchone()[0] == 8
print('sqlite-first-use: 20 rounds x 8 simultaneous first adds passed')
