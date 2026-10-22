#!/usr/bin/env python3
"""Check usage pages against CLI help and MCP tools/list; never call board tools."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default=str(ROOT / 'target/release/agentboard'))
args = parser.parse_args()

# Only usage pages are scanned. Historical client fixtures (notably
# tests/schema-version.py) are excluded by path, never by retired tool name.
paths = [ROOT / name for name in ('README.md', 'PROTOCOL.md')]
for directory in ('skills/agentboard-manager', 'skills/manager', 'docs'):
    paths.extend(sorted((ROOT / directory).rglob('*.md')))

# An explicit scratch path overrides any caller AGENTBOARD_DB. The path is
# nonexistent for either backend; tools/list needs no board operations.
with tempfile.TemporaryDirectory(prefix='agentboard-doc-names-') as scratch:
    with subprocess.Popen([args.binary, '--db', str(Path(scratch) / 'board')],
                          stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE, text=True) as proc:
        def rpc(seq, method, params):
            proc.stdin.write(json.dumps(dict(jsonrpc='2.0', id=seq, method=method,
                                            params=params)) + '\n')
            proc.stdin.flush()
            while True:
                line = proc.stdout.readline()
                assert line, 'MCP process closed before replying'
                reply = json.loads(line)
                if reply.get('id') == seq:
                    assert 'result' in reply, reply
                    return reply['result']
        try:
            rpc(1, 'initialize', dict(protocolVersion='2025-11-25', capabilities={},
                                    clientInfo=dict(name='doc-names', version='1')))
            proc.stdin.write(json.dumps(dict(jsonrpc='2.0',
                                            method='notifications/initialized')) + '\n')
            proc.stdin.flush()
            tools = {tool['name'] for tool in rpc(2, 'tools/list', {})['tools']}
        finally:
            proc.stdin.close()
            proc.wait(timeout=5)

help_cache = {}
def commands(prefix):
    if prefix not in help_cache:
        result = subprocess.run([args.binary, *prefix, '--help'],
                                capture_output=True, text=True, timeout=5, check=True)
        section = result.stdout.split('Commands:\n', 1)
        help_cache[prefix] = set(re.findall(r'^  ([a-z][a-z-]*)\s',
                                  section[1].split('\nOptions:', 1)[0], re.M)) if len(section) == 2 else set()
    return help_cache[prefix]

errors = []
cli_count = tool_count = 0
for path in paths:
    fenced = False
    for number, line in enumerate(path.read_text().splitlines(), 1):
        if line.lstrip().startswith('```'):
            fenced = not fenced
        # Names of fields and Rust internals are not MCP invocations. Match
        # standalone tool-shaped names, including bare names in prose/tables.
        for match in re.finditer(r'\b(?:task|epic|diff|session|board)_[a-z_]+\b', line):
            name = match.group()
            if name.endswith('_id') or line[match.end():].startswith('.'):
                continue
            if line[max(0, match.start()-2):match.start()] == '::':
                continue
            if path.name == 'surrealdb-migration.md' and name in {'session_with', 'board_path'}:
                continue  # Rust function discussed in migration implementation history.
            tool_count += 1
            if name not in tools:
                errors.append(f'{path.relative_to(ROOT)}:{number}: unknown MCP tool {name}')
        # Full CLI examples, both fenced shell and inline code. Walk nested
        # groups using help only; stop at arguments/options/placeholders.
        examples = re.findall(r'`([^`]+)`', line)
        # Playbook shorthand such as `manager down` and `diff add` uses
        # the same grammar as examples with an explicit executable.
        # These diagnostic messages are prose, not board subcommands.
        examples = [example for example in examples
                    if not example.startswith(('board server unavailable:',
                                               'board write lock busy'))]
        examples = [
            'agentboard ' + example
            if example.split() and example.split()[0] in commands(())
            else example
            for example in examples
        ]
        if fenced and re.match(r'^\s*(?:[~./\w-]*/)?agentboard\s', line):
            examples.append(line.split('#', 1)[0])
        for match in re.finditer(r'(?<![\w/-])(?:[~./\w-]*/)?agentboard[ \t]+([^`\n]+)', '\n'.join(examples)):
            words = re.findall(r'[a-z][a-z-]*|[^\s]+', match.group(1))
            prefix = ()
            for word in words:
                choices = commands(prefix)
                if not choices or word.startswith(('-', '<', '[', '$')):
                    break
                if not re.fullmatch('[a-z][a-z-]*', word):
                    break  # prose or an argument, not a command
                cli_count += 1
                if word not in choices:
                    errors.append(f'{path.relative_to(ROOT)}:{number}: unknown CLI command {" ".join((*prefix, word))}')
                    break
                prefix = (*prefix, word)

if errors:
    raise SystemExit('\n'.join(errors))
print(f'doc-names: {len(paths)} pages, {cli_count} CLI names, {tool_count} MCP names passed')
