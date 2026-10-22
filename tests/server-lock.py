#!/usr/bin/env python3
"""Managed daemon lifetime flock and upgrade exclusion, on host scratch storage.
python3 tests/server-lock.py BINARY [ServerLockTests.test_name]
"""
import fcntl
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time
import unittest
import urllib.request

binary = str(Path(sys.argv.pop(1)).resolve())

def endpoint():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return f'127.0.0.1:{s.getsockname()[1]}'

class ServerLockTests(unittest.TestCase):
    def setUp(self):
        self.scratch = tempfile.TemporaryDirectory(prefix='agentboard-server-lock-')
        self.board = Path(self.scratch.name) / 'data'
        self.url = endpoint()
        self.env = dict(os.environ, AGENTBOARD_DB=str(self.board), AGENTBOARD_SURREAL_URL=self.url)
        self.pid = None
        self.extra_pids = set()

    def cli(self, *args, url=None):
        env = dict(self.env, AGENTBOARD_SURREAL_URL=url or self.url)
        return subprocess.run([binary, *args], env=env, text=True, capture_output=True, timeout=30)

    def start(self):
        result = self.cli('add', 'managed scratch task')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.pid = int((self.board / 'board.lock').read_text())
        self.assertNotEqual(self.pid, os.getpid())
        self.assertIn(f'surrealkv://{self.board}'.encode(), Path(f'/proc/{self.pid}/cmdline').read_bytes().split(b'\0'))
        urllib.request.urlopen(f'http://{self.url}/health', timeout=1).close()
        # CLI exited, but the actual daemon retains the flock, not just a wrapper.
        with open(self.board / 'board.lock', 'r+') as lock:
            with self.assertRaises(BlockingIOError):
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        # The lifetime lock must not deadlock ordinary per-operation writers.
        again = self.cli('add', 'second managed task')
        self.assertEqual(again.returncode, 0, again.stderr)

    def stop(self):
        if self.pid is None:
            return
        try:
            os.killpg(self.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        # Wait for kernel lock release, even if the daemon becomes an orphan zombie.
        for _ in range(100):
            with open(self.board / 'board.lock', 'r+') as lock:
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    self.pid = None
                    return
                except BlockingIOError:
                    time.sleep(.05)
        self.fail('daemon lock survived its process group')

    def tearDown(self):
        # Also clean up a second daemon if a mutation wrongly let it start.
        for pid in self.extra_pids:
            try:
                os.killpg(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        self.stop()
        self.scratch.cleanup()

    def test_second_server_refuses_same_directory(self):
        self.start()
        other_url = endpoint()
        result = self.cli('ls', url=other_url)
        observed = int((self.board / 'board.lock').read_text())
        if observed != self.pid:
            self.extra_pids.add(observed)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn('board store lock busy', result.stderr)
        self.assertIn(f'held by pid {self.pid}', result.stderr)
        self.assertIn(str(self.board / 'board.lock'), result.stderr)
        with socket.socket() as s:
            self.assertNotEqual(s.connect_ex(tuple([other_url.split(':')[0], int(other_url.split(':')[1])])), 0)

    def test_upgrade_refuses_locked_store(self):
        self.start()
        # Different port avoids the positive-port check: the flock must decide.
        result = self.cli('board', 'upgrade', '--owner', url=endpoint())
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn('board store lock busy', result.stderr)
        self.assertIn(f'held by pid {self.pid}', result.stderr)

    def test_upgrade_proceeds_when_lock_free(self):
        self.start()
        self.stop()
        result = self.cli('board', 'upgrade', '--owner')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('board upgrade: version 2 -> 2', result.stdout)
        self.assertIn('committed atomically', result.stdout)
        with open(self.board / 'board.lock', 'r+') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

if __name__ == '__main__':
    unittest.main()
