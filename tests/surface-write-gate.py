#!/usr/bin/env python3
"""Compile stub seats and mutation-check actual MCP writes, on scratch boards only.

Copies sources to TMPDIR; never patches the checkout, installs, or uses a live board.
Shares Cargo's dependency cache. Requires SurrealDB 2.7.x, just like cargo test.
"""
import json
import os
import pathlib
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent
COUNT = len(re.findall(r'^\s*#\[tool\(', (ROOT / 'src/server.rs').read_text(), re.M))
assert COUNT != 19, 'update the pre-E11 stub fixture'


class Seat:
    def __init__(self, binary, env):
        self.proc = subprocess.Popen([str(binary)], stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                     text=True, env=env)
        self.seq = 0
        self.rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
                               'clientInfo': {'name': 'stub-seat', 'version': '0'}})
        self.proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'method': 'notifications/initialized'}) + '\n')
        self.proc.stdin.flush()

    def rpc(self, method, params):
        self.seq += 1
        self.proc.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': self.seq,
                                         'method': method, 'params': params}) + '\n')
        self.proc.stdin.flush()
        while True:
            line = self.proc.stdout.readline()
            assert line, 'seat closed stdout'
            reply = json.loads(line)
            if reply.get('id') == self.seq:
                return reply

    def call(self, tool, **args):
        return self.rpc('tools/call', {'name': tool, 'arguments': args})

    def close(self):
        self.proc.stdin.close()
        self.proc.wait(timeout=5)
        self.proc.stdout.close()


def text(reply):
    return '\n'.join(c.get('text', '') for c in reply.get('result', {}).get('content', []))


def failed(reply):
    return 'error' in reply or reply.get('result', {}).get('isError', False)


def build(tree, backend, dest):
    args = ['cargo', 'build', '--locked', '--quiet']
    if backend == 'sqlite':
        args += ['--no-default-features', '--features', 'sqlite']
    env = dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target'))
    result = subprocess.run(args, cwd=tree, env=env, capture_output=True, text=True, timeout=180)
    assert result.returncode == 0, result.stderr
    shutil.copy2(ROOT / 'target/debug/agentboard', dest)


def probe(matching, stub, env):
    owner = Seat(matching, env)
    stale = Seat(stub, env)
    try:
        reply = owner.call('task_add', title='scratch surface gate')
        assert not failed(reply), reply
        task = re.search(r'T\d+', text(reply))[0]
        before = owner.call('task_get', id=task, full=True)
        # Registration is deliberately first: failure must precede that first write.
        writes = [('session_register', {'name': 'stale', 'agent': 'codex'}),
                  ('task_claim', {'id': task}),
                  ('task_update', {'id': task, 'note': 'must not land'}),
                  ('task_update', {'id': task, 'status': 'working'}),
                  ('diff_add', {'id': task, 'body': 'must not land'}),
                  ('task_add', {'title': 'must not land'}),
                  ('epic_add', {'title': 'must not land'})]
        for name, args in writes:
            reply = stale.call(name, **args)
            diagnostic = json.dumps(reply)
            assert failed(reply), f'stale seat reached first write: {name}: {reply}'
            assert 'seat tools=19' in diagnostic and f'board tools={COUNT}' in diagnostic, reply
            assert 'reinstall the seat binary' in diagnostic and 'reload the seat' in diagnostic, reply
        assert text(owner.call('diff_list', id=task)) == '(no diffs)'
        after = owner.call('task_get', id=task, full=True)
        # No claim, note, diff or status event may have landed.
        assert re.findall(r'#\d+', text(before)) == re.findall(r'#\d+', text(after))
        reply = owner.call('session_register', name='matching', agent='codex')
        assert not failed(reply), reply
    finally:
        stale.close()
        owner.close()


def review_probe(binary, env):
    # This client holds only the old verbs. The current binary lets the request
    # reach apply; a 19-count binary is stopped earlier by the separate probe.
    seat = Seat(binary, env)
    old_tools = {
        'board_brief', 'epic_add', 'epic_edit', 'epic_get', 'epic_list',
        'epic_update', 'session_list', 'session_register', 'session_send',
        'session_tail', 'task_add', 'task_ask', 'task_children', 'task_claim',
        'task_edit', 'task_get', 'task_list', 'task_update', 'task_wait',
    }
    assert len(old_tools) == 19
    assert not {'diff_add', 'diff_get', 'diff_list'} & old_tools

    def call(name, **args):
        assert name in old_tools, f'pre-E11 client cannot call {name}'
        return seat.call(name, **args)

    try:
        parent = call('task_add', title='scratch parent')
        assert not failed(parent), parent
        parent_id = re.search(r'T\d+', text(parent))[0]
        reply = call('task_add', title='scratch pre-gate submission', parent=parent_id)
        assert not failed(reply), reply
        task = re.search(r'T\d+', text(reply))[0]
        reply = call('task_claim', id=task)
        assert not failed(reply), reply
        before = call('task_get', id=task, full=True)
        reply = call('task_update', id=task, status='review', report='must not land')
        diagnostic = json.dumps(reply)
        assert failed(reply), f'no-diff review accepted: {reply}'
        assert f'{task} cannot move to review: no diff recorded since it was last claimed' in diagnostic, reply
        assert f'agentboard diff add {task}' in diagnostic, reply
        assert f'diff_add id={task}' in diagnostic, reply
        after = call('task_get', id=task, full=True)
        # Refusal must precede all patch writes; avoid relative-age assertions.
        assert re.findall(r'#\d+', text(before)) == re.findall(r'#\d+', text(after))
        assert f'{task} working ' in text(after), after
        assert 'must not land' not in text(after), after
    finally:
        seat.close()


def run_checks(review_binary=None):
    with tempfile.TemporaryDirectory(prefix='agentboard-write-gate-') as directory:
        scratch = pathlib.Path(directory)
        tree = scratch / 'source'
        tree.mkdir()
        for name in ['Cargo.toml', 'Cargo.lock', 'build.rs']:
            shutil.copy2(ROOT / name, tree / name)
        shutil.copytree(ROOT / 'src', tree / 'src')
        original = {name: (tree / name).read_text() for name in
                    ['src/ident.rs', 'src/store/sqlite/mod.rs', 'src/store/surreal/mod.rs',
                     'src/store/surreal/tasks.rs']}
        backends = ['surreal', 'sqlite']
        if review_binary:
            identity = subprocess.check_output([str(review_binary), 'board-build'], text=True)
            backends = [identity.split('backend=')[1].strip()]
        for backend in backends:
            for name, source in original.items():
                (tree / name).write_text(source)
            matching, stub, mutant = [scratch / f'{backend}-{n}' for n in ['matching', 'stub', 'mutant']]
            if review_binary:
                matching = review_binary
            else:
                build(tree, backend, matching)
            ident = re.sub(r'pub fn surface_tools\(\) -> i64 \{.*?\n\}',
                           'pub fn surface_tools() -> i64 { 19\n}', original['src/ident.rs'], flags=re.S)
            assert ident != original['src/ident.rs']
            (tree / 'src/ident.rs').write_text(ident)
            if not review_binary:
                build(tree, backend, stub)
            env = dict(os.environ, AGENTBOARD_DB=str(scratch / f'{backend}-board'),
                       AGENTBOARD_ACTOR='surface-test')
            server = None
            log = None
            try:
                if backend == 'surreal':
                    surreal = shutil.which('surreal') or str(pathlib.Path.home() / '.surrealdb/surreal')
                    with socket.socket() as sock:
                        sock.bind(('127.0.0.1', 0))
                        port = sock.getsockname()[1]
                    url = f'127.0.0.1:{port}'
                    env.update(AGENTBOARD_SURREAL_URL=url, AGENTBOARD_SURREAL_USER='root',
                               AGENTBOARD_SURREAL_PASS='root')
                    log = (scratch / 'server.log').open('w')
                    server = subprocess.Popen([surreal, 'db', 'start', '--no-banner', '--log', 'warn',
                                               '--bind', url, '--username', 'root', '--password', 'root',
                                               f'surrealkv://{scratch / "data"}'], stdout=log, stderr=log)
                    for _ in range(100):
                        try:
                            urllib.request.urlopen(f'http://{url}/health', timeout=.2).close()
                            break
                        except OSError:
                            time.sleep(.1)
                    else:
                        raise AssertionError('scratch server never became healthy')
                else:
                    env.pop('AGENTBOARD_SURREAL_URL', None)
                review_probe(matching, env)
                print(f'pre-gate-client: {backend}: no-diff review refused by name; no patch writes landed', flush=True)
                if review_binary:
                    continue
                # Delete only the review gate, retaining the surface guard.
                gatepath = f'src/store/{backend}/tasks.rs'
                gatesource = (tree / gatepath).read_text()
                deleted = gatesource.replace('if submit && !diff_since_claim(', 'if false && !diff_since_claim(')
                assert deleted != gatesource
                (tree / gatepath).write_text(deleted)
                (tree / 'src/ident.rs').write_text(original['src/ident.rs'])
                build(tree, backend, mutant)
                try:
                    review_probe(mutant, env)
                except AssertionError as error:
                    assert 'no-diff review accepted:' in str(error), error
                else:
                    raise AssertionError('deleted review gate survived the negative test')
                print(f'pre-gate-client: {backend}: deleting diff gate makes no-diff review test fail', flush=True)
                (tree / gatepath).write_text(gatesource)
                (tree / 'src/ident.rs').write_text(ident)
                probe(matching, stub, env)
                print(f'surface-write-gate: {backend}: compiled 19-tool stub refused before registration, claim, note, status, diff, task and epic writes; matching seat accepted', flush=True)
                # Delete all count comparisons on the write path in the scratch copy.
                if backend == 'sqlite':
                    path = 'src/store/sqlite/mod.rs'
                    value = original[path].replace('if client.is_none() || client != board {', 'if false {')
                else:
                    path = 'src/store/surreal/mod.rs'
                    value = original[path].replace('&& board != crate::ident::surface_tools()', '&& false')
                    txpath = 'src/store/surreal/tasks.rs'
                    tx = original[txpath].replace('IF {count} = NULL OR $surface = NONE OR $surface != {count}', 'IF false')
                    assert tx != original[txpath]
                    (tree / txpath).write_text(tx)
                assert value != original[path]
                (tree / path).write_text(value)
                build(tree, backend, mutant)
                try:
                    probe(matching, mutant, env)
                except AssertionError as error:
                    assert 'stale seat reached first write: session_register' in str(error), error
                else:
                    raise AssertionError('deleted comparison survived the negative test')
                print(f'surface-write-gate: {backend}: deleting comparison makes first-write refusal test fail', flush=True)
            finally:
                if server is not None:
                    server.terminate()
                    server.wait(timeout=5)
                if log is not None:
                    log.close()
    if not review_binary:
        print('surface-write-gate: both backends and mutation checks passed')


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--review-only':
        run_checks(pathlib.Path(sys.argv[2]).resolve())
    else:
        try:
            run_checks()
        finally:
            # A cached mutant must never remain as the checkout's runnable debug binary.
            revision = subprocess.check_output(['git', 'describe', '--always', '--dirty', '--abbrev=8'],
                                               cwd=ROOT, text=True).strip()
            subprocess.run(['cargo', 'build', '--locked', '--quiet'], cwd=ROOT,
                           env=dict(os.environ, CARGO_TARGET_DIR=str(ROOT / 'target'),
                                    AGENTBOARD_BUILD_COMMIT=revision),
                           check=True, timeout=180)
