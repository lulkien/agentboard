#!/usr/bin/env python3
"""Seat-held catalog proof; no server, board, build or installation required."""
import json
import pathlib
import re
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parent.parent
check = root / 'scripts/check-seat-surface.py'
source = check.read_text()
names = re.findall(r'name = "([a-z_]+)"', (root / 'src/server.rs').read_text())
count = len(re.findall(r'^\s*#\[tool\(', (root / 'src/server.rs').read_text(), re.M))
assert len(names) == count, names
head = subprocess.check_output(['git', 'describe', '--always', '--dirty', '--abbrev=8'],
                               cwd=root, text=True).strip()


def run(code, catalog, commit=head, advertised=count):
    surface = {'build': f'agentboard 0.1.0 tools={advertised} commit={commit} backend=surreal',
               'tools': catalog}
    return subprocess.run([sys.executable, '-c',
                           f'__file__ = {str(check)!r}\n' + code, '--mcp-surface'],
                          input=json.dumps(surface), capture_output=True, text=True, timeout=5)


def rejects(code, catalog, **kwargs):
    result = run(code, catalog, **kwargs)
    assert result.returncode == 1 and 'seat surface refused:' in result.stderr, result
    return result.stderr


assert run(source, names).returncode == 0
legacy = [n for n in names if n not in {'diff_add', 'diff_get', 'diff_list', 'board_build'}]
assert f'got tools={len(legacy)}' in rejects(source, legacy)
rejects(source, names, commit='00000000')
rejects(source, names, advertised=count - 1)
for missing in ['diff_add', 'diff_get', 'diff_list', 'board_build']:
    rejects(source, ['replacement' if n == missing else n for n in names])
rejects(source, names[:-1] + [names[0]])

# Delete each guard in memory; its negative proof must fail with the guard gone.
catalog_mutant = re.sub(r'    if len\(names\).*?\nelse:', '\nelse:', source, flags=re.S)
identity_mutant = re.sub(r'if \(not commit.*?\n    sys.exit\(f\'seat surface refused: want tools=.*?\n',
                         '', source, flags=re.S)
for mutant, catalog, kwargs in [(catalog_mutant, legacy, {}),
                                (identity_mutant, names, {'commit': '00000000'})]:
    assert mutant != source
    try:
        rejects(mutant, catalog, **kwargs)
    except AssertionError:
        assert run(mutant, catalog, **kwargs).returncode == 0
    else:
        raise AssertionError('negative proof survived deleted guard')

# All installed entry points use the same three steps; Codex/Hermes share SKILL.md.
steps = []
for name in ['SKILL.md', 'claude.md', 'opencode.md']:
    text = (root / 'skills/manager' / name).read_text()
    steps.append(text[text.index('1. '):])
assert len(set(steps)) == 1
assert re.findall(r'^\d+\.', steps[0], re.M) == ['1.', '2.', '3.']
assert 'surface first' in steps[0] and 'Coder and reviewer' in steps[0]
# Fixed-count scan, and only that. It reads exactly the five seat-facing pages
# listed below: skills/agentboard-manager/{roles,SKILL}.md and the three
# skills/manager entry files. It does NOT read AGENTS.md (its prove table prints
# the expected tool count beside the rg command on purpose), README.md,
# PROTOCOL.md, workflow.md or docs/. No scan there means no coverage there: a
# literal on those pages is not caught by this test.
for name in ['skills/agentboard-manager/roles.md', 'skills/agentboard-manager/SKILL.md',
             'skills/manager/SKILL.md', 'skills/manager/claude.md',
             'skills/manager/opencode.md']:
    text = (root / name).read_text()
    published = re.findall(r'(?:\*\*)?(\d+)(?:\*\*)? tools|tools=(\d+)', text)
    assert not published, f'{name}: fixed count must be replaced by the derived value: {published}'
    assert 'derived' in text
print('seat-surface: published counts use derived output; no fixed count can drift')
print('seat-surface: fixed-count scan covers roles.md, agentboard-manager/SKILL.md and the three '
      'skills/manager entry files; AGENTS.md and the other docs are not scanned')
print('seat-surface: matching catalog accepted; pre-E11, missing diff verbs, wrong count and stale commit refused; both deleted guards detected; three shared steps match')
