#!/usr/bin/env python3
"""v1 worktree binary + new binary; scratch storage and random loopback ports only.
Usage: python3 tests/board-upgrade.py OLD_BINARY NEW_BINARY
"""
import base64
import json
import http.server
import queue
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
    env = dict(os.environ, AGENTBOARD_DB=str(board), AGENTBOARD_SURREAL_URL=endpoint)
    processes = []
    owner = None

    def cli(binary, *args, ok=True):
        p = subprocess.run([binary, *args], env=env, text=True, capture_output=True, timeout=30)
        assert (p.returncode == 0) == ok, (args, p.stdout, p.stderr)
        return p.stdout if ok else p.stderr

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
        return {t: sql(f'SELECT * FROM {t} ORDER BY id;')[0]['result']
                for t in ('tasks', 'epics', 'events', 'diffs', 'sessions', 'counters')}

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
        refusal = cli(new, 'get', 'T1', ok=False)
        assert 'board upgrade --owner' in refusal and 'restart seats' in refusal and 'mv ' not in refusal
        assert snapshot() == before, 'implicit open must leave rows untouched'
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
        assert 'build' not in sql('INFO FOR TABLE meta;')[0]['result']['fields']
        # A non-stamp meta row proves backfill independently of stamp UPSERTs.
        sql('CREATE meta:legacy SET version=1;')
        meta_before = sql('SELECT * FROM meta ORDER BY id;')[0]['result']
        marker_url = f'http://127.0.0.1:{marker.server_port}/stamp?before='
        sql("DEFINE EVENT pause_upgrade ON TABLE meta WHEN $after.version = 2 AND $before.version = 1 "
            f"THEN http::get('{marker_url}' + $before.build);")
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
            assert reached.get(timeout=10) == '/stamp?before='
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
        assert 'build' not in sql('INFO FOR TABLE meta;')[0]['result']['fields']
        assert sql('SELECT * FROM meta ORDER BY id;')[0]['result'] == meta_before
        sql('REMOVE EVENT pause_upgrade ON TABLE meta;')
        stop(server)
        output = cli(new, 'board', 'upgrade', '--owner')
        print(output)
        for table, rows in before.items():
            assert f'rows {table}: {len(rows)} -> {len(rows)}' in output, output
        assert "DEFINE FIELD IF NOT EXISTS build ON meta TYPE string DEFAULT '';" in output
        assert "UPDATE meta SET build = build ?? '' WHERE build IS NONE;" in output
        assert 'additive definitions checked (IF NOT EXISTS):' in output
        assert 'backfill missing defaults only:' in output
        server = start()
        assert 'build' in sql('INFO FOR TABLE meta;')[0]['result']['fields']
        assert sql('SELECT build FROM meta:legacy;')[0]['result'] == [{'build': ''}]
        after = snapshot()
        assert after == before, 'every stored row must remain byte-identical in canonical JSON'
        for table, rows in before.items():
            print(f'preserved {table}: {len(rows)} -> {len(after[table])}')
        assert json.dumps(before['tasks'][0], sort_keys=True).encode() == json.dumps(after['tasks'][0], sort_keys=True).encode()
        stamps = sql('SELECT * FROM meta:schema; SELECT * FROM meta:opened; SELECT * FROM meta:writer;')
        upgrading_build = output.split('generation=2 ', 1)[1].splitlines()[0]
        for stamp in stamps:
            assert stamp['result'][0]['version'] == 2
            assert stamp['result'][0]['build'] == upgrading_build
            assert 'commit=' in upgrading_build and 'tools=23' in upgrading_build
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
            assert 'treating as not a holder' in warned and 'version 2 -> 2' in warned, warned
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
        assert 'version 2 -> 2' in again
        server = start()
        assert snapshot() == before
        sql('UPDATE meta:schema SET version=99;')
        stop(server)
        assert 'supports only version 1' in cli(new, 'board', 'upgrade', '--owner', ok=False)
        server = start()
        assert sql('SELECT version FROM meta:schema;')[0]['result'][0]['version'] == 99
        assert snapshot() == before
        stop(server)
        print('board-upgrade: owner gate, fail-closed open, both server refusals, unreadable process warned and continued, pure-v1 DDL/backfill rollback and retry, output asserted, killed-owner child dead, cleanup descendant dead, all rows preserved, stamps, v1 refused, v2 accepted, idempotent retry passed')
    finally:
        if owner is not None and owner.poll() is None:
            owner.kill()
            owner.communicate(timeout=5)
        for p in processes:
            if p.poll() is None:
                stop(p)
