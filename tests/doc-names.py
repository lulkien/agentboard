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
paths = [ROOT / name for name in ('README.md', 'PROTOCOL.md', 'AGENTS.md',
                                 'skills/README.md', 'seats/README.md')]
for directory in sorted((ROOT / 'skills').iterdir()):
    if (directory / 'SKILL.md').exists():
        paths.extend(sorted(directory.rglob('*.md')))

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
command_cache = {}

def command_names(output):
    section = output.split('Commands:\n', 1)
    return set(re.findall(r'^  ([a-z][a-z-]*)\s',
                          section[1].split('\nOptions:', 1)[0], re.M)) if len(section) == 2 else set()

def help_output(prefix):
    if prefix not in help_cache:
        result = subprocess.run([args.binary, *prefix, '--help'],
                                capture_output=True, text=True, timeout=5, check=True)
        help_cache[prefix] = result.stdout
    return help_cache[prefix]

def commands(prefix):
    if prefix not in command_cache:
        output = help_output(prefix)
        command_cache[prefix] = command_names(output)
    return command_cache[prefix]

def flags(prefix):
    output = help_output(prefix)
    section = output.split('Options:\n', 1)
    if len(section) != 2:
        return set()
    return set(re.findall(r'--[a-z][a-z-]*', section[1]))

errors = []
cli_count = tool_count = 0

# Keep the documented Task record in sync with its serialized Rust fields.
domain_source = (ROOT / 'src/domain.rs').read_text()
task_match = re.search(r'pub struct Task\s*\{([^}]*)\}', domain_source, re.S)
if not task_match:
    errors.append('src/domain.rs: cannot find pub struct Task')
else:
    serde_fields = re.findall(r'^\s*pub\s+([a-z][a-z0-9_]*):', task_match.group(1), re.M)
    protocol = (ROOT / 'PROTOCOL.md').read_text()
    documented_match = re.search(r'A task is a record:\s*((?:`[^`]+`(?:,\s*)?)+)', protocol)
    if not documented_match:
        errors.append('PROTOCOL.md: cannot find the task record field list')
    else:
        documented_fields = re.findall(r'`([^`]+)`', documented_match.group(1))
        if documented_fields != serde_fields:
            errors.append('PROTOCOL.md: task record fields do not match src/domain.rs Task serde fields; '
                          f'documented {documented_fields}, serde {serde_fields}')

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
                if word.startswith(('-', '<', '[', '$')):
                    break
                choices = commands(prefix)
                if not choices:
                    break
                if not re.fullmatch('[a-z][a-z-]*', word):
                    break  # prose or an argument, not a command
                cli_count += 1
                if word not in choices:
                    errors.append(f'{path.relative_to(ROOT)}:{number}: unknown CLI command {" ".join((*prefix, word))}')
                    break
                prefix = (*prefix, word)

# A stale number and a stale list are different failures: seats/README.md once
# said "nine tools" and named nine of eleven, so a count-only check passed while
# the list was wrong. Both are judged against the tools/list reply fetched above.
NUMBER_WORDS = {word: value for value, word in enumerate(
    'zero one two three four five six seven eight nine ten eleven twelve'.split())}
TOOL_LIST = re.compile(r'^[ \t]*-[ \t]+The board tool set is the (\w+) tools of this build:',
                       re.M)
for path in paths:
    text = path.read_text()
    relative = path.relative_to(ROOT)
    for match in TOOL_LIST.finditer(text):
        # The list wraps across lines, so read to the end of the bullet.
        block = ''
        for following in text[match.end():].splitlines(keepends=True):
            stripped = following.strip()
            if not stripped or stripped.startswith(('- ', '* ', '#')):
                break
            block += following
        line_number = text.count('\n', 0, match.start()) + 1
        stated = NUMBER_WORDS.get(match.group(1).lower())
        if stated is None and match.group(1).isdigit():
            stated = int(match.group(1))
        if stated is None:
            errors.append(f'{relative}:{line_number}: unrecognised tool count '
                          f'{match.group(1)!r}')
        elif stated != len(tools):
            errors.append(f'{relative}:{line_number}: says {match.group(1)} tools, '
                          f'the router exposes {len(tools)}')
        named = []
        for name in re.finditer(r'`([^`]+)`', block):
            located = text.count('\n', 0, match.end() + name.start()) + 1
            named.append(name.group(1))
            if name.group(1) not in tools:
                errors.append(f'{relative}:{located}: the tool list names {name.group(1)}, '
                              'which the router does not expose')
        for missing in sorted(tools - set(named)):
            errors.append(f'{relative}:{line_number}: the tool list is missing {missing}')

if errors:
    raise SystemExit('\n'.join(errors))
print(f'doc-names: {len(paths)} pages, {cli_count} CLI names, {tool_count} MCP names passed')
