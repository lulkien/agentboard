#!/usr/bin/env python3
"""Derive published tool counts from the compiled MCP router, without board calls."""
import argparse
from pathlib import Path
import re
import sys

root = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(root / 'scripts'))
from router_tools import tool_names
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', default=str(root / 'target/release/agentboard'))
parser.add_argument('--check', action='store_true')
args = parser.parse_args()
count = len(tool_names(args.binary))

changed = []
words = ['zero', 'one', 'two', 'three', 'four', 'five', 'six', 'seven', 'eight',
         'nine', 'ten', 'eleven', 'twelve']
for name in ['src/server.rs', 'README.md', 'PROTOCOL.md', 'AGENTS.md',
             'skills/agentboard-team/SKILL.md', 'seats/README.md']:
    path = root / name
    before = path.read_text()
    after = re.sub(r'MCP surface: \d+ tools', f'MCP surface: {count} tools', before)
    after = re.sub(r'\b\d+ MCP tools', f'{count} MCP tools', after)
    after = re.sub(r'MCP surface has \d+ tools', f'MCP surface has {count} tools', after)
    after = re.sub(r'MCP router: \d+ tools', f'MCP router: {count} tools', after)
    word = words[count] if count < len(words) else str(count)
    # Spelled-out counts only count when a count follows them: "the eleven tools
    # of this build" is a published count, "nine" on its own is prose. One pass,
    # so a rewritten word is never rewritten again.
    spelled = '|'.join(w.capitalize() for w in words) + '|' + '|'.join(words)
    after = re.sub(rf'\b({spelled})(?=\s+(?:MCP\s+)?tools)',
                   lambda match: word.capitalize() if match.group(1)[0].isupper() else word,
                   after)
    if after != before:
        changed.append(name)
        if not args.check:
            path.write_text(after)
if args.check and changed:
    raise SystemExit('stale tool count: ' + ', '.join(changed))
print(f'MCP router: {count} tools; published counts ' + ('match' if args.check else 'updated'))
