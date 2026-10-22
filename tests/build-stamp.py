#!/usr/bin/env python3
"""Read-only regression proof, against a build with AGENTBOARD_BUILD_COMMIT=00000000."""
import json
import os
import time
import pathlib
import subprocess
import sys
import tempfile

root = pathlib.Path(__file__).resolve().parent.parent
binary = str(pathlib.Path(sys.argv[1]).resolve())
line = subprocess.check_output([binary, 'board-build'], text=True).strip()
assert 'commit=00000000 (stale, HEAD is ' in line, line
request = {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
    'protocolVersion': '2025-11-25', 'capabilities': {},
    'clientInfo': {'name': 'stamp-test', 'version': '0'}}}
reply = subprocess.run([binary], input=json.dumps(request) + '\n',
                       text=True, capture_output=True, timeout=5, check=True)
assert line in reply.stdout, reply.stdout
check = root / 'scripts/check-seat-surface.py'
refused = subprocess.run([sys.executable, check, binary], capture_output=True, text=True)
assert refused.returncode != 0 and 'seat surface refused:' in refused.stderr, refused
# Even an older binary without freshness annotations must fail the seat check.
head = subprocess.check_output(['git', 'describe', '--always', '--dirty', '--abbrev=8'],
                               cwd=root, text=True).strip()
import re
tools = len(re.findall(r'^\s*#\[tool\(', (root / 'src/server.rs').read_text(), re.M))
with tempfile.TemporaryDirectory(prefix='agentboard-stamp-') as scratch:
    fake = pathlib.Path(scratch) / 'binary'
    for count, commit, expected in [
        (tools, '00000000', 1),
        (tools + 1, head, 1),
        (tools, head + ' (unverified, build checkout unavailable)', 1),
        (tools, head + ' (stale, HEAD is other)', 1),
        (tools, head, 0),
    ]:
        fake.write_text(f'#!/bin/sh\necho "agentboard 0.1.0 tools={count} commit={commit} backend=surreal"\n')
        fake.chmod(0o700)
        result = subprocess.run([sys.executable, check, fake], capture_output=True, text=True)
        assert result.returncode == expected, result
    # A hung display git is killed as a process group and degrades to unverified.
    git = pathlib.Path(scratch) / 'git'
    pidfile = pathlib.Path(scratch) / 'pid'
    git.write_text('#!/bin/sh\necho $$ > "$AGENTBOARD_TEST_GIT_PID"\nexec /bin/sleep 30\n')
    git.chmod(0o700)
    env = dict(os.environ, PATH=scratch, AGENTBOARD_DELIVER_TIMEOUT_MS='50',
               AGENTBOARD_TEST_GIT_PID=str(pidfile))
    timed = subprocess.check_output([binary, 'board-build'], text=True, env=env, timeout=3)
    assert '(unverified, build checkout unavailable)' in timed, timed
    unverified = subprocess.run([binary], input=json.dumps(request) + '\n',
                                text=True, capture_output=True, timeout=3, env=env, check=True)
    assert '(unverified, build checkout unavailable)' in unverified.stdout, unverified.stdout
    pid = int(pidfile.read_text())
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError('timed-out git must be dead')
    started = time.monotonic()
    refused = subprocess.run([sys.executable, check, fake], env=env,
                             capture_output=True, text=True, timeout=4)
    assert refused.returncode == 1 and 'seat surface refused:' in refused.stderr, refused
    assert 'Traceback' not in refused.stderr and time.monotonic() - started < 4, refused
print('build-stamp: stale CLI and MCP identities explicit; seat rejects stale commit and wrong tool count; matching surface accepted')
print('build-stamp: unverified surface refused; display git timeout kills process; seat git timeout refuses cleanly')
