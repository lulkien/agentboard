#!/usr/bin/env python3
"""Check overlap across the five core pages listed in PAGES below.

Scope: README.md, PROTOCOL.md, AGENTS.md, and the agentboard-manager SKILL.md,
workflow.md and roles.md. Paths resolve from this script's repository.
Headings are excluded because navigation labels are not instructions. Table and
list markers are excluded from exact-line checks because structural rows and
bullets are not standalone prose; their text still participates in shingles.
Code fences and their contents are excluded because diagrams and code are not
prose rules. Lines under ten words are excluded from exact-line checks because
short labels and Markdown wrapping produce incidental matches; their words
still participate in shingles, so wrapping cannot hide a duplicated rule.
Shingles span prose lines and ignore heading, table, list and fence markers.
Any exact duplicate line or shared 10/12/16-word shingle across pages fails.
"""
from itertools import combinations
from pathlib import Path
import re

PAGES = (
    'README.md',
    'PROTOCOL.md',
    'AGENTS.md',
    'skills/agentboard-manager/SKILL.md',
    'skills/agentboard-manager/workflow.md',
    'skills/agentboard-manager/roles.md',
)


def extract(text):
    lines = set()
    words = []
    fence = None
    for raw in text.splitlines():
        line = raw.strip()
        marker = re.match(r'^(`{3,}|~{3,})', line)
        if marker:
            token = marker.group()
            if fence is None:
                fence = token
            elif token[0] == fence[0] and len(token) >= len(fence):
                fence = None
            continue
        if fence or re.match(r'^#{1,6}\s', line):
            continue
        structural = line.startswith('|') or re.match(r'^(?:[-+*]|\d+[.)])\s', line)
        if not structural and len(line.split()) >= 10:
            lines.add(line)
        prose = re.sub(r'^(?:[-+*]|\d+[.)])\s+', '', line)
        words.extend(word for word in prose.split() if word != '|')
    return lines, words


def main():
    root = Path(__file__).resolve().parent.parent
    pages = {p: extract((root / p).read_text()) for p in PAGES}
    failures = 0
    for size in (None, 10, 12, 16):
        entries = {}
        for page, (lines, words) in pages.items():
            entries[page] = lines if size is None else {
                ' '.join(words[i:i + size]) for i in range(len(words) - size + 1)
            }
        count = 0
        label = 'exact duplicate lines' if size is None else f'shared {size}-word shingles'
        for left, right in combinations(PAGES, 2):
            for phrase in sorted(entries[left] & entries[right]):
                print(f'FAIL {label}: {left} <-> {right}: {phrase}')
                count += 1
        print(f'{label}: {count}')
        failures += count
    return int(failures > 0)


if __name__ == '__main__':
    raise SystemExit(main())
