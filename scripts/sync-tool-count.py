#!/usr/bin/env python3
"""Derive published tool counts from the compiled MCP router, without board calls."""
import argparse
import json
from pathlib import Path
import re
import subprocess

root = Path(__file__).resolve().parent.parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default=str(root / 'target/release/agentboard'))
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
proc = subprocess.Popen([args.binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=subprocess.PIPE, text=True)

def rpc(seq, method, params):
    proc.stdin.write(json.dumps(dict(jsonrpc='2.0', id=seq, method=method, params=params)) + '\n')
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
                              clientInfo=dict(name='tool-count', version='1')))
    proc.stdin.write(json.dumps(dict(jsonrpc='2.0', method='notifications/initialized')) + '\n')
    proc.stdin.flush()
    count = len(rpc(2, 'tools/list', {})['tools'])
finally:
    proc.stdin.close()
    proc.wait(timeout=5)
    proc.stdout.close()
    proc.stderr.close()

changed = []
words = ['zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight', 'nine', 'ten']
for name in ['src/server.rs', 'README.md', 'PROTOCOL.md', 'AGENTS.md',
             'skills/agentboard-team/SKILL.md']:
    path = root / name
    before = path.read_text()
    after = re.sub(r'MCP surface: \d+ tools', f'MCP surface: {count} tools', before)
    after = re.sub(r'\b\d+ MCP tools', f'{count} MCP tools', after)
    after = re.sub(r'MCP surface has \d+ tools', f'MCP surface has {count} tools', after)
    after = re.sub(r'MCP router: \d+ tools', f'MCP router: {count} tools', after)
    word = words[count] if count < len(words) else str(count)
    after = re.sub(r'\b(?:Nine|Ten)(?= (?:MCP )?tools)', word.capitalize(), after)
    after = re.sub(r'\b(?:nine|ten)(?= (?:MCP )?tools)', word, after)
    if after != before:
        changed.append(name)
        if not args.check:
            path.write_text(after)
if args.check and changed:
    raise SystemExit('stale tool count: ' + ', '.join(changed))
print(f'MCP router: {count} tools; published counts ' + ('match' if args.check else 'updated'))
