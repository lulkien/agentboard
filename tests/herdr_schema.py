"""Derive manager request fields, then check the installed herdr schema.

This is a deliberately small Rust token reader, not a method inventory. It
supports the request construction forms in src/manager and refuses unresolved
params/method expressions rather than silently omitting a new call site.
Only request shapes are read: no manager verb is executed.
"""
import json
import os
from pathlib import Path
import re
import signal
import socket
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
TOKEN = re.compile(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\])*"|[A-Za-z_][A-Za-z_0-9]*|\S', re.S)


def tokens(text):
    return [m[0] for m in TOKEN.finditer(text) if not m[0].startswith(('//', '/*'))]


def end(ts, start):
    stack = []
    for i in range(start, len(ts)):
        if ts[i] in ('(', '[', '{'):
            stack.append({'(': ')', '[': ']', '{': '}'}[ts[i]])
        elif ts[i] in (')', ']', '}'):
            assert stack and stack.pop() == ts[i], 'unbalanced Rust tokens'
            if not stack:
                return i
    raise AssertionError('unclosed Rust expression')


def split(ts, delimiter=','):
    parts, start, i = [], 0, 0
    while i < len(ts):
        if ts[i] in ('(', '[', '{'):
            i = end(ts, i)
        elif ts[i] == delimiter:
            parts.append(ts[start:i])
            start = i + 1
        i += 1
    if ts[start:]:
        parts.append(ts[start:])
    return parts


def string(token):
    assert token.startswith('"'), f'expected a literal, got {token}'
    return json.loads(token)


def functions(text):
    ts = tokens(text)
    # Exclude fake requests in test modules, without hiding later production code.
    i = 0
    while i < len(ts):
        if ts[i:i + 7] == ['#', '[', 'cfg', '(', 'test', ')', ']']:
            start = ts.index('{', i + 7)
            del ts[i:end(ts, start) + 1]
        else:
            i += 1
    for i, token in enumerate(ts):
        if token == 'fn':
            start = ts.index('{', i)
            yield ts[i + 1], ts[start + 1:end(ts, start)]


def fields(expr, body, resolving=()):
    """Static field paths; [] denotes an array element, not a field name."""
    if len(expr) == 1 and re.fullmatch(r'\w+', expr[0]):
        name = expr[0]
        assert name not in resolving, f'cyclic params expression {name}'
        for i in range(len(body) - 1):
            if body[i] == 'let':
                stop = i + 1
                while stop < len(body) and body[stop] not in ('=', ';'):
                    stop += 1
                declaration = body[i + 1:stop]
                if declaration and declaration[0] == 'mut':
                    declaration = declaration[1:]
                if declaration and declaration[0] == name and body[stop] == '=':
                    rhs = split(body[stop + 1:], ';')[0]
                    paths = fields(rhs, body, (*resolving, name))
                    # Optional fields set after json! construction.
                    for j in range(len(body) - 4):
                        if body[j:j + 2] == [name, '['] and body[j + 3:j + 5] == [']', '=']:
                            key = string(body[j + 2])
                            paths.add((key,))
                            rhs = split(body[j + 5:], ';')[0]
                            if rhs[:3] == ['json', '!', '('] and rhs[3:4] == ['{']:
                                paths.update((key, *p) for p in fields(rhs, body, (*resolving, name)))
                    if 'Vec' in declaration:
                        paths = {('[]', *p) for p in paths}
                    return paths
        raise AssertionError(f'unresolved params expression {name}')
    result, found = set(), False
    i = 0
    while i < len(expr):
        if expr[i:i + 3] == ['json', '!', '(']:
            close = end(expr, i + 2)
            obj = expr[i + 3:close]
            assert obj and obj[0] == '{', 'request json! must construct an object'
            for entry in split(obj[1:-1]):
                assert len(entry) >= 3 and entry[1] == ':', f'unsupported JSON entry {entry}'
                key = string(entry[0])
                result.add((key,))
                value = entry[2:]
                # Follow locally built arrays of JSON objects (subscriptions).
                if len(value) == 1 and value[0] not in resolving:
                    is_local = any(body[j:j + 2] == ['let', value[0]] for j in range(len(body) - 1))
                    if is_local:
                        result.update((key, *p) for p in fields(value, body, resolving))
            found = True
            i = close
        i += 1
    if not found:
        # A serializable struct literal, e.g. layout.apply's Params {tab_id, root}.
        if len(expr) > 2 and expr[1] == '{' and end(expr, 1) == len(expr) - 1:
            for entry in split(expr[2:-1]):
                assert len(entry) == 1 or entry[1] == ':', f'unsupported struct entry {entry}'
                result.add((entry[0],))
        else:
            raise AssertionError(f'unsupported params expression: {" ".join(expr)}')
    return result


def inventory():
    sources = {p: p.read_text() for p in sorted((ROOT / 'src/manager').glob('*.rs'))}
    client = sources[ROOT / 'src/manager/client.rs']
    # The subscription wrapper's wire name is defined in its own body.
    subscribe = dict(functions(client))['subscribe']
    at = subscribe.index('METHOD')
    sub_method = string(subscribe[subscribe.index('=', at) + 1])
    used = {}
    for path, text in sources.items():
        for name, body in functions(text):
            for i in range(len(body) - 2):
                if body[i] != '.' or body[i + 2] != '(':
                    continue
                operation = body[i + 1]
                if operation not in ('call', 'call_for', 'subscribe', 'exchange'):
                    continue
                args = split(body[i + 3:end(body, i + 2)])
                if operation == 'exchange':
                    # Internal forwarding is generic; pin's literal is a request.
                    if not args[2][0].startswith('"'):
                        continue
                    method, params = string(args[2][0]), args[3]
                elif operation == 'subscribe':
                    method, params = sub_method, args[0]
                else:
                    assert len(args[0]) == 1, f'{path}:{name}: nonliteral method'
                    method, params = string(args[0][0]), args[1]
                try:
                    used.setdefault(method, set()).update(fields(params, body))
                except AssertionError as error:
                    raise AssertionError(f'{path.name}:{name}: {method}: {error}') from error
    assert used, 'no manager requests extracted'
    return used


def resolve(schema, node):
    while '$ref' in node:
        ref = node['$ref']
        assert ref.startswith('#/'), f'nonlocal schema ref {ref}'
        node = schema
        for key in ref[2:].split('/'):
            node = node[key.replace('~1', '/').replace('~0', '~')]
    return node


def has_path(schema, node, path):
    node = resolve(schema, node)
    if not path:
        return True
    if any(has_path(schema, part, path) for kind in ('oneOf', 'anyOf', 'allOf') for part in node.get(kind, [])):
        return True
    key = 'items' if path[0] == '[]' else 'properties'
    child = node.get(key) if key == 'items' else node.get(key, {}).get(path[0])
    return child is not None and has_path(schema, child, path[1:])


def compare(schema, used):
    protocol = schema['protocol']
    methods = {v['properties']['method']['const']: v['properties']['params']
               for v in schema['schemas']['request']['oneOf']}
    for method, paths in sorted(used.items()):
        assert method in methods, f'herdr schema drift: missing method {method} (protocol {protocol})'
        for path in sorted(paths):
            assert has_path(schema, methods[method], path), (
                f'herdr schema drift: missing param {method}.{".".join(path)} (protocol {protocol})')
    return len(methods), len({m.split('.')[0] for m in methods})


def reachable():
    explicit = next((os.environ[k] for k in ('AGENTBOARD_HERDR_SOCK', 'HERDR_SOCKET') if os.environ.get(k)), None)
    defaults = [Path(os.environ.get('XDG_CONFIG_HOME', '/nonexistent')) / 'herdr/herdr.sock',
                Path(os.environ.get('HOME', '/nonexistent')) / '.config/herdr/herdr.sock']
    path = Path(explicit) if explicit else next((p for p in defaults if p.exists()), defaults[-1])
    try:
        with socket.socket(socket.AF_UNIX) as sock:
            sock.settimeout(2)
            sock.connect(str(path))
            sock.sendall(b'{"id":"schema-guard","method":"ping","params":{}}\n')
            with sock.makefile('rb') as stream:
                reply = json.loads(stream.readline(65536))
        return reply['result']['protocol']
    except OSError as error:
        message = f'herdr_schema: needs reachable herdr control socket at {path}: {error}'
        assert os.environ.get('AGENTBOARD_REQUIRE_HERDR') != '1', message
        print(f'SKIP {message} (set AGENTBOARD_REQUIRE_HERDR=1 to require it)')
        return None


def live(used):
    protocol = reachable()
    if protocol is None:
        return
    with tempfile.TemporaryDirectory(prefix='agentboard-schema-') as tmp:
        output = Path(tmp) / 'schema.json'
        bounded(['herdr', 'api', 'schema', '--output', str(output)], 10)
        schema = json.loads(output.read_text())
    assert schema['protocol'] == protocol, f'schema protocol {schema["protocol"]} differs from live protocol {protocol}'
    total, groups = compare(schema, used)
    print(f'schema inventory: {total} methods in {groups} groups, protocol {protocol}')
    print(f'schema ok: {len(used)} methods, {sum(map(len, used.values()))} param fields, protocol {protocol}')


def bounded(command, timeout):
    # Tests only: bound the schema CLI and its process group.
    with subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                          start_new_session=True) as child:
        try:
            stdout, stderr = child.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(child.pid, signal.SIGKILL)
            child.communicate()
            raise AssertionError(f'{command[0]} timed out after {timeout}s') from None
        assert child.returncode == 0, (stdout + stderr).decode()


def self_test(used):
    body = tokens('let mut p = json!({"first": x}); p["optional"] = x;')
    assert fields(['p'], body) == {('first',), ('optional',)}
    assert fields(tokens('Params {tab_id: tab, root: &root}'), []) == {('tab_id',), ('root',)}
    try:
        fields(['unknown'], [])
    except AssertionError:
        pass
    else:
        raise AssertionError('unresolved params silently ignored')
    fixture = '#[cfg(test)] mod tests { fn fake() { c.call("fake", json!({})); } } fn real() { c.call("real", json!({})); }'
    assert [name for name, _ in functions(fixture)] == ['real']
    nested = tokens('let subs: Vec<Value> = xs.map(|x| json!({"field": x})).collect();')
    assert fields(tokens('json!({"subscriptions": subs})'), nested) == {('subscriptions',), ('subscriptions', '[]', 'field')}
    # Compare real derived shapes with a synthetic schema; extras are allowed.
    variants = []
    for method, paths in {**used, 'future.extra': set()}.items():
        node = {}
        for path in paths:
            target = node
            for field in path:
                target = target.setdefault('items', {}) if field == '[]' else target.setdefault('properties', {}).setdefault(field, {})
        variants.append({'properties': {'method': {'const': method}, 'params': node}})
    schema = {'protocol': 22, 'schemas': {'request': {'oneOf': variants}}}
    compare(schema, used)
    for mutated in ({**used, 'pane.nonexistent': set()}, {**used, 'agent.read': {('renamed_field',)}}):
        try:
            compare(schema, mutated)
        except AssertionError as error:
            assert 'protocol 22' in str(error) and ('pane.nonexistent' in str(error) or 'renamed_field' in str(error))
        else:
            raise AssertionError('drift was not detected')
    print('source/schema checks ok: optional/struct fields, unknown expression refusal, extra method allowed, missing method/field refused')
    with tempfile.TemporaryDirectory(prefix='agentboard-schema-absent-') as tmp:
        env = dict(os.environ, HOME=tmp, XDG_CONFIG_HOME=tmp,
                   HERDR_SOCKET='', AGENTBOARD_HERDR_SOCK=str(Path(tmp) / 'missing.sock'))
        for strict in ('0', '1'):
            env['AGENTBOARD_REQUIRE_HERDR'] = strict
            result = subprocess.run([sys.executable, __file__], env=env,
                                    capture_output=True, text=True, timeout=5)
            assert (result.returncode == 0) == (strict == '0'), result.stdout + result.stderr
            assert ('SKIP herdr_schema:' in result.stdout) == (strict == '0')
            assert 'needs reachable herdr control socket' in result.stdout + result.stderr
    print('socket guard: absent socket skips; strict switch fails by name')
    with tempfile.TemporaryDirectory(prefix='agentboard-schema-kill-') as tmp:
        pidfile = Path(tmp) / 'pids'
        program = ('import os,subprocess,time; from pathlib import Path; '
                   'p=subprocess.Popen(["sleep","30"]); '
                   f'Path({str(pidfile)!r}).write_text(str(os.getpid())+" "+str(p.pid)); '
                   'time.sleep(30)')
        try:
            bounded([sys.executable, '-c', program], 1)
        except AssertionError as error:
            assert 'timed out' in str(error)
        else:
            raise AssertionError('deadline did not fire')
        for pid in pidfile.read_text().split():
            stat = Path('/proc') / pid / 'stat'
            try:
                state = stat.read_text().split(') ')[1][0]
            except (FileNotFoundError, ProcessLookupError):
                continue  # Reaped before or during the /proc read.
            assert state == 'Z', f'process {pid} survived group kill'
    print('schema CLI deadline: parent and descendant are dead')


if __name__ == '__main__':
    try:
        used = inventory()
        if '--inventory' in sys.argv:
            for method, paths in sorted(used.items()):
                print(f'{method}: {", ".join(".".join(p) for p in sorted(paths)) or "(none)"}')
        elif '--self-test' in sys.argv:
            self_test(used)
        else:
            live(used)
    except (AssertionError, KeyError, ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
