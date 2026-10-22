#!/usr/bin/env python3
"""v1 worktree binary + new binary; scratch storage and random loopback ports only.
Usage: python3 tests/board-upgrade.py OLD_BINARY NEW_BINARY
"""
import base64
import json
import http.server
import queue
import re
import signal
import shlex
import threading
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

old, new = map(lambda p: str(Path(p).resolve()), sys.argv[1:])
surreal = shutil.which('surreal') or str(Path.home() / '.surrealdb/surreal')

def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]

with tempfile.TemporaryDirectory(prefix='agentboard-upgrade-') as scratch:
    root = Path(scratch)
    board = root / 'data'
    endpoint = f'127.0.0.1:{port()}'
    env = dict(os.environ, AGENTBOARD_DB=str(board), AGENTBOARD_SURREAL_URL=endpoint, AGENTBOARD_ACTOR='upgrade-test')
    processes = []
    owner = None

    def cli(binary, *args, ok=True):
        p = subprocess.run([binary, *args], env=env, text=True, capture_output=True, timeout=30)
        assert (p.returncode == 0) == ok, (args, p.stdout, p.stderr)
        return p.stdout + p.stderr if ok else p.stderr

    def start(url=endpoint):
        log = open(root / f'server-{len(processes)}.log', 'w')
        p = subprocess.Popen([surreal, 'db', 'start', '--no-banner', '--log', 'warn',
                              '--bind', url, '--username', 'root', '--password', 'root',
                              f'surrealkv://{board}'], stdout=log, stderr=log)
        processes.append(p)
        for _ in range(100):
            try:
                urllib.request.urlopen(f'http://{url}/health', timeout=.2).close()
                return p
            except OSError:
                assert p.poll() is None, (root / f'server-{len(processes)-1}.log').read_text()
                time.sleep(.1)
        raise AssertionError('server startup timed out')

    def stop(p, kill=False):
        if kill:
            p.kill()
        else:
            p.terminate()
        p.wait(timeout=10)

    def sql(query):
        req = urllib.request.Request(f'http://{endpoint}/sql', query.encode(), headers={
            'Authorization': 'Basic ' + base64.b64encode(b'root:root').decode(),
            'surreal-ns': 'agentboard', 'surreal-db': 'board', 'Accept': 'application/json'})
        try:
            with urllib.request.urlopen(req, timeout=40) as r:
                result = json.load(r)
        except urllib.error.HTTPError as error:
            raise AssertionError(error.read().decode()) from error
        assert all(r['status'] == 'OK' for r in result), result
        return result

    def snapshot():
        result = {t: sql(f'SELECT * FROM {t} ORDER BY id;')[0]['result']
                  for t in ('tasks', 'epics', 'events', 'diffs', 'sessions', 'counters')}
        # The additive reporter column is checked separately; old fields must
        # remain byte-identical across the version transition.
        for row in result['tasks']:
            row.pop('reporter', None)
        return result

    try:
        assert 'requires --owner' in cli(new, 'board', 'upgrade', ok=False)
        assert not board.exists(), 'seat refusal must not create a board'
        server = start()
        cli(old, 'epic', 'add', 'preserved epic', '--description', 'epic bytes')
        cli(old, 'add', 'preserved task', '--epic', 'E1', '--description', 'line 1\nline 2 — café', '--goal', 'preserve me')
        cli(old, 'note', 'T1', 'event bytes')
        cli(old, 'diff', 'add', 'T1', '--body', 'verbatim diff bytes')
        cli(old, 'register', 'scratch-seat', '--agent', 'human', '--task', 'T1')
        before = snapshot()
        assert len(before['tasks']) == 1 and len(before['epics']) == 1 and len(before['events']) == 4
        first = cli(new, 'get', 'T1')
        assert 'preserved task' in first and 'reporter: upgrade-test' in first
        assert 'migrated v1 -> v3:' in first
        assert snapshot() == before, 'additive migration must preserve all stored rows'
        assert sql('SELECT version FROM meta:schema;')[0]['result'][0]['version'] == 3
        # Reset a scratch v1 fixture to exercise offline interruption/exclusion.
        sql('REMOVE FIELD reviews ON diffs; UPDATE meta:schema SET version=1;')
        assert 'server running at' in cli(new, 'board', 'upgrade', '--owner', ok=False)
        stop(server)
        # A server using the same data at another port must also be named/refused.
        other = start(f'127.0.0.1:{port()}')
        assert 'board held by' in cli(new, 'board', 'upgrade', '--owner', ok=False)
        stop(other)
        # This external HTTP marker is reached from the actual stamp event,
        # after additive DDL/backfill inside the transaction. Hold its response
        # until the owner is killed; startup speed cannot satisfy this check.
        reached = queue.Queue()
        release_marker = threading.Event()
        class StampMarker(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                reached.put(self.path)
                release_marker.wait(20)
                self.send_response(200)
                self.end_headers()
            def log_message(self, *_):
                pass
        marker = http.server.ThreadingHTTPServer(('127.0.0.1', 0), StampMarker)
        marker.daemon_threads = True
        threading.Thread(target=marker.serve_forever, daemon=True).start()
        server = start()
        assert 'reviews' not in sql('INFO FOR TABLE diffs;')[0]['result']['fields']
        # A non-stamp meta row proves backfill independently of stamp UPSERTs.
        sql('CREATE meta:legacy SET version=1;')
        meta_before = sql('SELECT * FROM meta ORDER BY id;')[0]['result']
        marker_url = f'http://127.0.0.1:{marker.server_port}/stamp?before='
        sql("DEFINE EVENT pause_upgrade ON TABLE meta WHEN $after.version = 3 AND $before.version = 1 "
            f"THEN http::get('{marker_url}' + <string>$before.version);")
        stop(server)
        # Permit only this loopback marker in the test server; production
        # startup arguments and capabilities remain unchanged.
        marker_bin = root / 'marker-bin'
        marker_bin.mkdir()
        marker_wrapper = marker_bin / 'surreal'
        marker_wrapper.write_text('#!/bin/sh\nexec ' + shlex.quote(surreal) +
                                  ' "$@" --allow-net=127.0.0.1:' + str(marker.server_port) + '\n')
        marker_wrapper.chmod(0o700)
        marker_env = dict(env, PATH=str(marker_bin) + ':' + env['PATH'])
        owner = subprocess.Popen([new, 'board', 'upgrade', '--owner'], env=marker_env,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        child_pid = None
        for _ in range(2000):
            for task in Path(f'/proc/{owner.pid}/task').glob('*'):
                try:
                    children = (task / 'children').read_text().split()
                except OSError:
                    continue
                if children:
                    child_pid = int(children[0])
                    break
            if child_pid:
                break
            assert owner.poll() is None, owner.communicate()
            time.sleep(.001)
        assert child_pid, 'must observe the actual private server before killing owner'
        # Parse and wait for the private endpoint, then require the transaction's
        # external marker. Unlike an elapsed delay, receipt proves the event ran.
        for _ in range(100):
            private_args = Path(f'/proc/{child_pid}/cmdline').read_bytes().split(b'\0')
            if b'--bind' in private_args:
                break
            assert owner.poll() is None, owner.communicate()
            time.sleep(.01)
        private_url = private_args[private_args.index(b'--bind') + 1].decode()
        for _ in range(100):
            try:
                urllib.request.urlopen(f'http://{private_url}/health', timeout=.2).close()
                break
            except OSError:
                assert owner.poll() is None, owner.communicate()
                time.sleep(.05)
        else:
            raise AssertionError('private server health never reached')
        try:
            assert reached.get(timeout=10) == '/stamp?before=1'
            assert owner.poll() is None, owner.communicate()
        except queue.Empty:
            owner.kill()
            raise AssertionError(f'stamp event marker not reached: {owner.communicate()}')
        owner.kill()
        owner.communicate(timeout=5)
        release_marker.set()
        marker.shutdown()
        marker.server_close()
        for _ in range(100):
            try:
                state = Path(f'/proc/{child_pid}/stat').read_text().split(') ')[1][0]
                if state == 'Z':
                    break
            except FileNotFoundError:
                break
            time.sleep(.05)
        else:
            os.kill(child_pid, 9)
            raise AssertionError('private server survived killed upgrader')
        server = start()
        assert sql('SELECT version FROM meta:schema;')[0]['result'][0]['version'] == 1
        assert snapshot() == before
        assert 'reviews' not in sql('INFO FOR TABLE diffs;')[0]['result']['fields']
        assert sql('SELECT * FROM meta ORDER BY id;')[0]['result'] == meta_before
        sql('REMOVE EVENT pause_upgrade ON TABLE meta;')
        stop(server)
        output = cli(new, 'board', 'upgrade', '--owner')
        print(output)
        for table, rows in before.items():
            assert f'rows {table}: {len(rows)} -> {len(rows)}' in output, output
        assert "DEFINE FIELD IF NOT EXISTS reviews ON diffs TYPE option<int>;" in output
        assert "WHERE" in output and "IS NONE" in output
        assert 'additive definitions checked (IF NOT EXISTS):' in output
        assert 'backfill missing defaults only:' in output
        server = start()
        assert 'reviews' in sql('INFO FOR TABLE diffs;')[0]['result']['fields']
        assert sql('SELECT version FROM meta:legacy;')[0]['result'] == [{'version': 1}]
        after = snapshot()
        assert after == before, 'every stored row must remain byte-identical in canonical JSON'
        for table, rows in before.items():
            print(f'preserved {table}: {len(rows)} -> {len(after[table])}')
        assert json.dumps(before['tasks'][0], sort_keys=True).encode() == json.dumps(after['tasks'][0], sort_keys=True).encode()
        assert sql('SELECT version FROM meta:schema;')[0]['result'] == [{'version': 3}]
        assert sql('SELECT * FROM meta:opened; SELECT * FROM meta:writer;')[0]['result'] == []
        assert 'schema version=3' in output
        assert 'want version 1' in cli(old, 'get', 'T1', ok=False)
        assert 'preserved task' in cli(new, 'get', 'T1')
        stop(server)
        # A non-dumpable process is unknown evidence: warn by name, then proceed.
        # It is not a managed server and holds no lifetime flock.
        hidden = subprocess.Popen([sys.executable, '-c',
            "import ctypes,sys,time; "
            "f=open(sys.argv[1], 'w'); libc=ctypes.CDLL(None); "
            "assert libc.prctl(15,b'hidden-holder',0,0,0)==0; "
            "assert libc.prctl(4,0,0,0,0)==0; "
            "print('ready',flush=True); time.sleep(60)", str(board / 'held-file')],
            stdout=subprocess.PIPE, text=True, start_new_session=True)
        try:
            assert hidden.stdout.readline().strip() == 'ready'
            try:
                list(Path(f'/proc/{hidden.pid}/fd').iterdir())
            except PermissionError:
                pass
            else:
                raise AssertionError('fixture must make live holder descriptors unreadable')
            warned = cli(new, 'board', 'upgrade', '--owner')
            assert f'warning: cannot inspect pid {hidden.pid} (hidden-holder)' in warned, warned
            assert 'open fd directory' in warned and 'Permission denied' in warned, warned
            assert 'treating as not a holder' in warned and 'version 3 -> 3' in warned, warned
            assert f'uid {os.geteuid()} processes' in warned, warned
        finally:
            os.killpg(hidden.pid, signal.SIGKILL)
            hidden.wait(timeout=5)

        # A forked helper must die with the owned server on normal cleanup.
        wrapper_dir = root / 'bin'
        wrapper_dir.mkdir()
        wrapper = wrapper_dir / 'surreal'
        helper_pid_file = root / 'helper.pid'
        wrapper.write_text('#!/bin/sh\nsleep 60 &\necho $! > ' + shlex.quote(str(helper_pid_file)) +
                           '\nexec ' + shlex.quote(surreal) + ' "$@"\n')
        wrapper.chmod(0o700)
        original_path = env['PATH']
        env['PATH'] = str(wrapper_dir) + ':' + original_path
        again = cli(new, 'board', 'upgrade', '--owner')
        env['PATH'] = original_path
        helper_pid = int(helper_pid_file.read_text())
        for _ in range(100):
            try:
                if Path(f'/proc/{helper_pid}/stat').read_text().split(') ')[1][0] == 'Z':
                    break
            except FileNotFoundError:
                break
            time.sleep(.02)
        else:
            os.kill(helper_pid, 9)
            raise AssertionError('server descendant survived group cleanup')
        assert 'version 3 -> 3' in again
        server = start()
        assert snapshot() == before
        sql('UPDATE meta:schema SET version=99;')
        stop(server)
        assert 'supports versions 1..=3' in cli(new, 'board', 'upgrade', '--owner', ok=False)
        server = start()
        assert sql('SELECT version FROM meta:schema;')[0]['result'][0]['version'] == 99
        assert snapshot() == before
        stop(server)
        print('board-upgrade: owner gate, first-call additive migration, both server refusals, unreadable process warned and continued, pure-v1 DDL/backfill rollback and retry, output asserted, killed-owner child dead, cleanup descendant dead, all rows preserved, schema version, v1 refused, current accepted, idempotent retry passed')
    finally:
        if owner is not None and owner.poll() is None:
            owner.kill()
            owner.communicate(timeout=5)
        for p in processes:
            if p.poll() is None:
                stop(p)
