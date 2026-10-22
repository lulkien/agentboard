#!/usr/bin/env python3
"""Read-only seat-start check: installed identity must match this checkout."""
import json
import pathlib
import re
import subprocess
import sys

root = pathlib.Path(__file__).resolve().parent.parent
mcp_surface = len(sys.argv) == 2 and sys.argv[1] == '--mcp-surface'
binary = sys.argv[1] if len(sys.argv) > 1 else str(pathlib.Path.home() / '.cargo/bin/agentboard')
try:
    expected_commit = subprocess.check_output(
        ['git', 'describe', '--always', '--dirty', '--abbrev=8'],
        cwd=root, text=True, timeout=2, stderr=subprocess.DEVNULL
    ).strip()
except (subprocess.SubprocessError, OSError):
    sys.exit('seat surface refused: checkout revision unavailable (git failed or timed out)')
expected_tools = len(re.findall(r'^\s*#\[tool\(', (root / 'src/server.rs').read_text(), re.M))
if mcp_surface:
    # The seat supplies the catalog it actually holds, never a fresh server's list.
    try:
        surface = json.load(sys.stdin)
        line = surface['build']
        names = surface['tools']
        if not isinstance(line, str) or not isinstance(names, list) or not all(isinstance(n, str) for n in names):
            raise ValueError('build must be text and tools must be a list of names')
    except (ValueError, KeyError, TypeError) as error:
        sys.exit(f'seat surface refused: invalid MCP surface: {error}')
    required = {'board_build', 'diff_add', 'diff_get', 'diff_list'}
    if len(names) != expected_tools or len(set(names)) != len(names) or not required.issubset(names):
        sys.exit(f'seat surface refused: want tools={expected_tools} and {sorted(required)}; '
                 f'got tools={len(names)} missing={sorted(required - set(names))}')
else:
    line = subprocess.check_output([binary, 'board-build'], text=True, timeout=5).strip()
commit = re.search(r'\bcommit=(\S+)', line)
tools = re.search(r'\btools=(\d+)', line)
if (not commit or not tools or commit[1] != expected_commit
        or int(tools[1]) != expected_tools or '(stale,' in line or '(unverified,' in line):
    sys.exit(f'seat surface refused: want tools={expected_tools} commit={expected_commit}; got {line}')
print(f'seat surface ok: tools={expected_tools} commit={expected_commit}')
