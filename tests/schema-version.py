#!/usr/bin/env python3
"""Scratch-only CLI/MCP schema migrations, concurrency and deleted-guard proofs.

Set AGENTBOARD_SCHEMA_OLD_SQLITE to a d3b18e9 SQLite binary to also prove
new-board -> previous-v2 named refusal without writes.
AGENTBOARD_REPORTER_OLD_SQLITE/SURREAL optionally prove reporter attribution
against actual v2 binaries; all probes use this script's scratch boards.
"""
import base64
import concurrent.futures
import json
import os
import pathlib
import re
import shutil
import socket
import sqlite3
import sys
import subprocess
import tempfile
import threading
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent.parent

CURRENT = int(re.search(r'GENERATION: i64 = (\d+)', (ROOT / 'src/ident.rs').read_text()).group(1))

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
    # Keep matching/mutant artifacts private to this temporary run. In particular,
    # never copy a previous run's mutant out of the checkout's shared target.
    target = tree.parent / 'target'
    executable = target / 'debug/agentboard'
    # A successful build must produce this executable again, even when Cargo
    # would otherwise consider a previous matching/mutant build up to date.
    executable.unlink(missing_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    result = subprocess.run(args, cwd=tree, env=env, capture_output=True, text=True, timeout=600)
    assert result.returncode == 0, result.stderr
    assert executable.is_file(), f'build did not produce a fresh executable: {executable}'
    shutil.copy2(executable, dest)


def review_probe(binary, env):
    # This historical client has no diff verbs. The server must enforce the
    # submission gate regardless of the client's catalog; no count guard exists.
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
        assert f'{task} cannot move to review: no diff or evidence-only submission recorded since it was last claimed' in diagnostic, reply
        assert f'agentboard diff add {task}' in diagnostic, reply
        assert f'diff_add id={task}' in diagnostic, reply
        after = call('task_get', id=task, full=True)
        # Refusal must precede all patch writes; avoid relative-age assertions.
        assert re.findall(r'#\d+', text(before)) == re.findall(r'#\d+', text(after))
        assert f'{task} working ' in text(after), after
        assert 'must not land' not in text(after), after
    finally:
        seat.close()



def sqlite_rollback_probe(binary, old):
    with tempfile.TemporaryDirectory(prefix='agentboard-v2-rollback-') as directory:
        board = pathlib.Path(directory) / 'board'
        env = dict(os.environ, AGENTBOARD_DB=str(board))
        env.pop('AGENTBOARD_SURREAL_URL', None)

        def run(label, binary, *args):
            result = subprocess.run([str(binary), *args], env=env,
                                    capture_output=True, text=True, timeout=20)
            print(f'{label}: exit={result.returncode}\n{result.stdout}{result.stderr}'.rstrip(), flush=True)
            assert result.returncode == 0, result
            return result.stdout

        run('new writer creates fresh board', binary, 'add', 'old-read-probe')
        with sqlite3.connect(board) as conn:
            columns = {row[1] for row in conn.execute('PRAGMA table_info(board_stamp)')}
            assert {'surface_tools', 'opened_by', 'last_writer'} <= columns, columns
            stamp = conn.execute('SELECT generation,surface_tools,opened_by,last_writer FROM board_stamp').fetchone()
            assert stamp == (CURRENT, None, '', ''), stamp
            before = conn.execute('SELECT * FROM tasks').fetchall()
        for command in [('ls',), ('add', 'must-not-land')]:
            result = subprocess.run([str(old), *command], env=env, capture_output=True, text=True, timeout=20)
            assert result.returncode == 1 and 'binary is older than the board' in result.stderr, result
        with sqlite3.connect(board) as conn:
            assert conn.execute('SELECT * FROM tasks').fetchall() == before
            assert conn.execute('SELECT generation FROM board_stamp').fetchone() == (CURRENT,)
        print('sqlite rollback: old v2 refuses new board reads/writes without changing data', flush=True)



def checks(backend, binary, mutant, env, sql):
    def cli(*args):
        return subprocess.run([str(binary), *args], env=env, capture_output=True, text=True, timeout=20)

    def version(v):
        sql(f"UPDATE meta:schema SET version={v}" if backend == 'surreal' else
            f"UPDATE board_stamp SET generation={v}")

    def read_version():
        return sql('SELECT VALUE version FROM meta:schema')[0]['result'][0] if backend == 'surreal' else sql('SELECT generation FROM board_stamp')[0][0]

    # A long-lived MCP process must observe a version changed after its first call.
    seat = Seat(binary, env)
    try:
        reply = seat.call('task_add', title='preserved', description='exact original text')
        assert not failed(reply), reply
        task = re.search(r'T\d+', text(reply))[0]
        version(1)
        reply = seat.call('task_get', id=task, full=True)
        assert not failed(reply) and 'exact original text' in text(reply), reply
        assert read_version() == CURRENT
        # Missing fields are populated on the very first read, while present
        # values survive. This models the actual historical branch addition.
        expected_fills = 1 if backend == 'surreal' else sql('SELECT count(*) FROM tasks')[0][0]
        if backend == 'surreal':
            sql(f"REMOVE FIELD branch ON tasks; UPDATE tasks:{task[1:]} UNSET branch; UPDATE meta:schema SET version=1")
        else:
            sql("ALTER TABLE tasks DROP COLUMN branch")
            version(1)
        result = cli('get', task, '--full')
        assert result.returncode == 0 and 'branch: HEAD' in result.stdout, result
        assert re.search(rf"migrated v1 -> v{CURRENT}: \d+ definitions, {expected_fills} rows backfilled", result.stderr), result
        assert read_version() == CURRENT

        # Independent cold processes contend for the same migration lock.
        print(f'{backend}: {result.stderr.strip()}', flush=True)
        version(1)
        barrier = threading.Barrier(2)
        def read(_):
            barrier.wait()
            result = cli('get', task, '--full')
            assert result.returncode == 0 and 'exact original text' in result.stdout, result
            return result.stderr.count(f'migrated v1 -> v{CURRENT}:')
        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
            assert sum(pool.map(read, range(2))) == 1
        assert read_version() == CURRENT
        print(f'{backend}: concurrent first calls=2 succeeded; migrations=1; preserved rows verified', flush=True)

        version(CURRENT + 1)
        for tool, args in [('task_get', {'id': task, 'full': True}),
                           ('board_brief', {}), ('session_register', {'name': 'never-contacted', 'agent': 'human'})]:
            reply = seat.call(tool, **args)
            assert failed(reply) and 'binary is older than the board' in json.dumps(reply), reply
        result = cli('get', task)
        assert result.returncode == 1 and 'binary is older than the board' in result.stderr, result
        print(f'{backend}: {result.stderr.strip()}', flush=True)
        # Same negative test must fail with the version guard removed. The
        # mutant would actually serve this future board, so the test detects it.
        result = subprocess.run([str(mutant), 'get', task], env=env, capture_output=True, text=True, timeout=20)
        assert result.returncode == 0 and 'binary is older than the board' not in result.stderr, result
        assert read_version() == CURRENT + 1
        version(1)
        if backend == 'surreal':
            sql("DEFINE FIELD OVERWRITE title ON tasks TYPE int")
        else:
            sql("ALTER TABLE tasks RENAME COLUMN title TO legacy_title")
            sql("ALTER TABLE tasks ADD COLUMN title INTEGER")
        result = cli('get', task)
        assert result.returncode == 1 and 'non-additive type change' in result.stderr and 'manual step' in result.stderr, result
        assert read_version() == 1
    finally:
        seat.close()
    print(f'schema-version: {backend}: first CLI/MCP read preserves rows; concurrent calls migrate once; running MCP refuses future version; type change refuses; deleted comparison detected', flush=True)


def reporter_upgrade_probe(backend, binary, env, sql):
    """Use actual pre-reporter binaries when supplied; unit tests cover the same old shapes."""
    old = os.environ.get('AGENTBOARD_REPORTER_OLD_' + backend.upper())
    if not old:
        return
    def cli(executable, *args, ok=True):
        result = subprocess.run([str(executable), *args], env=env, capture_output=True, text=True, timeout=20)
        assert (result.returncode == 0) == ok, result
        return result
    for actor, title in [('analyzer', 'first'), ('coder', 'second'), ('unknown', 'no-history')]:
        cli(old, '--actor', actor, 'add', title, '--description', 'preserve exact original bytes')
    if backend == 'sqlite':
        sql("DELETE FROM events WHERE task_id=3 AND kind='add'")
        sql("INSERT INTO events(task_id,actor,kind,body,ts) VALUES(1,'imposter','add','later add',1)")
    else:
        sql("DELETE events WHERE task_id=3 AND kind='add'; CREATE events:999999 SET task_id=1, actor='imposter', kind='add', body='later add', ts=1")
    def preserved():
        if backend == 'sqlite':
            return sql('SELECT id,title,description,goal,status,assignee,parent_id,created_at,updated_at,report,note,priority FROM tasks ORDER BY id')
        return sql('SELECT id,title,description,goal,status,assignee,parent_id,created_at,updated_at,report,note,priority FROM tasks ORDER BY id')[0]['result']
    before = preserved()
    seat = Seat(binary, env)
    try:
        for task, reporter in [('T1', 'analyzer'), ('T2', 'coder'), ('T3', 'unknown (filed before this field existed)')]:
            reply = seat.call('task_get', id=task, full=True)
            assert not failed(reply) and f'reporter: {reporter}' in text(reply), reply
            result = cli(binary, 'get', task, '--full')
            assert f'reporter: {reporter}' in result.stdout, result
        assert preserved() == before
        for command in [('get', 'T1'), ('add', 'must-not-land')]:
            result = cli(old, *command, ok=False)
            assert 'binary is older than the board' in result.stderr, result
        # No public override exists, through either front door.
        assert failed(seat.call('task_update', id='T1', reporter='fake'))
        result = cli(binary, 'edit', 'T1', '--reporter', 'fake', ok=False)
        assert 'reporter' in result.stderr
        cli(binary, '--actor', 'other', 'edit', 'T1', '--title', 'renamed')
        assert 'reporter: analyzer' in cli(binary, 'get', 'T1').stdout
        # A stamped-current but missing field is a named refusal, also for an
        # already-running MCP process. Repair is modeled by a proper old stamp.
        if backend == 'sqlite':
            sql('ALTER TABLE tasks DROP COLUMN reporter')
        else:
            sql('REMOVE FIELD reporter ON tasks')
        reply = seat.call('task_get', id='T1')
        assert failed(reply) and 'missing tasks.reporter' in json.dumps(reply), reply
        result = cli(binary, 'get', 'T1', ok=False)
        assert 'missing tasks.reporter' in result.stderr and 'manual step' in result.stderr, result
        sql('UPDATE board_stamp SET generation=2' if backend == 'sqlite' else 'UPDATE meta:schema SET version=2')
        assert 'reporter: analyzer' in cli(binary, 'get', 'T1').stdout
    finally:
        seat.close()
    print(f'reporter-upgrade: {backend}: actual v2 fixture, earliest add, unknown, byte preservation, CLI/MCP equality, immutable identity and named missing-field refusal passed', flush=True)


def main():
    with tempfile.TemporaryDirectory(prefix='agentboard-schema-') as directory:
        scratch = pathlib.Path(directory)
        tree = scratch / 'source'
        tree.mkdir()
        for name in ['Cargo.toml', 'Cargo.lock']:
            shutil.copy2(ROOT / name, tree / name)
        shutil.copytree(ROOT / 'src', tree / 'src')
        for backend in ['surreal', 'sqlite']:
            matching, mutant = [scratch / f'{backend}-{n}' for n in ['matching', 'mutant']]
            build(tree, backend, matching)
            if backend == 'sqlite' and os.environ.get('AGENTBOARD_SCHEMA_OLD_SQLITE'):
                sqlite_rollback_probe(matching, pathlib.Path(os.environ['AGENTBOARD_SCHEMA_OLD_SQLITE']).resolve())
            path = tree / f'src/store/{backend}/mod.rs'
            source = path.read_text()
            if backend == 'sqlite':
                changed = source.replace('v > crate::ident::GENERATION', 'false')
                changed = changed.replace('if found == Some(crate::ident::GENERATION) {', 'if found.is_some_and(|v| v >= crate::ident::GENERATION) {')
            else:
                changed = source.replace('if found == Some(SCHEMA_VERSION) {', 'if found.is_some_and(|v| v >= SCHEMA_VERSION) {')
            assert changed != source
            path.write_text(changed)
            build(tree, backend, mutant)
            path.write_text(source)
            env = dict(os.environ, AGENTBOARD_DB=str(scratch / f'{backend}-board'), AGENTBOARD_ACTOR='schema-test')
            server = None
            try:
                if backend == 'surreal':
                    surreal = shutil.which('surreal') or str(pathlib.Path.home() / '.surrealdb/surreal')
                    with socket.socket() as sock:
                        sock.bind(('127.0.0.1', 0))
                        port = sock.getsockname()[1]
                    url = f'127.0.0.1:{port}'
                    env.update(AGENTBOARD_SURREAL_URL=url, AGENTBOARD_SURREAL_USER='root', AGENTBOARD_SURREAL_PASS='root')
                    server = subprocess.Popen([surreal, 'db', 'start', '--no-banner', '--log', 'warn', '--bind', url, '--username', 'root', '--password', 'root', f'surrealkv://{scratch / "data"}'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    for _ in range(100):
                        try:
                            urllib.request.urlopen(f'http://{url}/health', timeout=.2).close()
                            break
                        except OSError:
                            time.sleep(.1)
                    else:
                        raise AssertionError('scratch server never became healthy')
                    def sql(query):
                        req = urllib.request.Request(f'http://{url}/sql', query.encode(), headers={'Authorization': 'Basic ' + base64.b64encode(b'root:root').decode(), 'surreal-ns': 'agentboard', 'surreal-db': 'board', 'Accept': 'application/json'})
                        with urllib.request.urlopen(req, timeout=20) as response:
                            result = json.load(response)
                        assert all(r['status'] == 'OK' for r in result), result
                        return result
                else:
                    env.pop('AGENTBOARD_SURREAL_URL', None)
                    def sql(query):
                        with sqlite3.connect(env['AGENTBOARD_DB']) as conn:
                            return conn.execute(query).fetchall()
                reporter_upgrade_probe(backend, matching, env, sql)
                review_probe(matching, env)
                checks(backend, matching, mutant, env, sql)
            finally:
                if server:
                    server.terminate()
                    server.wait(timeout=5)
    print('schema-version: both backends and mutation checks passed')


def review_only(binary):
    with tempfile.TemporaryDirectory(prefix='agentboard-review-gate-') as directory:
        env = dict(os.environ, AGENTBOARD_DB=str(pathlib.Path(directory) / 'board'))
        env.pop('AGENTBOARD_SURREAL_URL', None)
        server = None
        try:
            if 'Data directory of the local board server' in subprocess.check_output([binary, '--help'], text=True):
                with socket.socket() as sock:
                    sock.bind(('127.0.0.1', 0))
                    url = f'127.0.0.1:{sock.getsockname()[1]}'
                env.update(AGENTBOARD_SURREAL_URL=url, AGENTBOARD_SURREAL_USER='root', AGENTBOARD_SURREAL_PASS='root')
                surreal = shutil.which('surreal') or str(pathlib.Path.home() / '.surrealdb/surreal')
                server = subprocess.Popen([surreal, 'db', 'start', '--no-banner', '--log', 'error', '--bind', url, '--username', 'root', '--password', 'root', 'memory'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                for _ in range(100):
                    try:
                        urllib.request.urlopen(f'http://{url}/health', timeout=.2).close()
                        break
                    except OSError:
                        time.sleep(.1)
                else:
                    raise AssertionError('scratch server never became healthy')
            review_probe(binary, env)
            print('pre-gate-client: no-diff review refused by name; no patch writes landed')
        finally:
            if server:
                server.terminate()
                server.wait(timeout=5)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--review-only':
        review_only(pathlib.Path(sys.argv[2]).resolve())
    else:
        try:
            main()
        finally:
            subprocess.run(['cargo', 'build', '--locked', '--quiet'], cwd=ROOT, check=True, timeout=180)
