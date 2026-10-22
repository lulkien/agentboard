#!/usr/bin/env bash
# Exercise installer destinations and all-or-nothing refusal in throwaway homes.
set -eu
cd "$(dirname "$0")/.."
python3 - <<'PY'
import os, pathlib, subprocess, tempfile
repo=pathlib.Path.cwd()
installer=repo/'skills/install.sh'
passed=0

def run(home, *args, **overrides):
    env=dict(os.environ,HOME=str(home))
    for key in ['CODEX_HOME','AGENTBOARD_CLAUDE_COMMANDS','AGENTBOARD_OPENCODE_COMMANDS']:
        env.pop(key,None)
    env.update({k:str(v) for k,v in overrides.items()})
    return subprocess.run([str(installer),*map(str,args)],env=env,capture_output=True,text=True)

def check(name):
    global passed
    passed+=1
    print('PASS',name)

with tempfile.TemporaryDirectory(prefix='agentboard-install-') as root:
    root=pathlib.Path(root)
    home=root/'fresh'
    p=run(home)
    assert p.returncode==0,p.stderr
    expected={
      '.hermes/skills/agentboard-manager':'agentboard-manager',
      '.claude/commands/manager.md':'commands/claude/manager.md',
      '.config/opencode/commands/manager.md':'commands/opencode/manager.md',
      '.codex/skills/manager':'commands/codex/manager',
      '.hermes/skills/manager':'commands/hermes/manager',
    }
    for dest,src in expected.items():
        path=home/dest
        assert path.is_symlink() and path.resolve()==repo/'skills'/src,(path,src)
    assert (home/'.codex/skills/manager/SKILL.md').is_file()
    assert (home/'.hermes/skills/manager/SKILL.md').is_file()
    assert not (home/'.codex/prompts/manager.md').exists()
    assert p.stdout.count('linked: ')==5,p.stdout
    # Follow installed symlinks as Hermes does: only its own manager is visible.
    managers=[]
    for directory, _, files in os.walk(home/'.hermes/skills', followlinks=True):
        if 'SKILL.md' in files:
            skill=pathlib.Path(directory)/'SKILL.md'
            if 'name: manager' in skill.read_text().split('---',2)[1].splitlines():
                managers.append(skill)
    assert managers==[home/'.hermes/skills/manager/SKILL.md'],managers
    assert not (home/'.hermes/skills/commands').exists()
    check('fresh install creates five repo links, with Codex and Hermes skill directories')

    p=run(home)
    assert p.returncode==1 and not p.stdout,p
    for dest in expected: assert f'exists: {home/dest} (use --force)' in p.stderr
    check('all existing destinations are named and refused without force')

    # A conflict at the LAST destination must not create any earlier parent.
    conflict=root/'conflict'
    existing=conflict/'.hermes/skills/manager'
    existing.mkdir(parents=True)
    (existing/'keep').write_text('untouched')
    before=existing.stat().st_mtime_ns
    p=run(conflict)
    assert p.returncode==1 and str(existing) in p.stderr and not p.stdout
    assert (existing/'keep').read_text()=='untouched' and existing.stat().st_mtime_ns==before
    assert not (conflict/'.codex').exists() and not (conflict/'.claude').exists()
    assert not (conflict/'.config').exists() and not (conflict/'.hermes/skills/agentboard-manager').exists()
    check('late conflict leaves earlier directories and existing contents untouched')

    broken=root/'broken'
    path=broken/'.codex/skills/manager'
    path.parent.mkdir(parents=True)
    path.symlink_to('missing')
    p=run(broken)
    assert p.returncode==1 and str(path) in p.stderr
    assert path.is_symlink() and os.readlink(path)=='missing'
    assert not (broken/'.hermes').exists()
    check('dangling destination symlink is refused without side effects')

    p=run(conflict,'--force')
    assert p.returncode==0,p.stderr
    assert existing.is_symlink() and (existing/'SKILL.md').is_file()
    check('force replaces the conflicting destination with its repo link')

    custom=root/'custom'
    codehome=root/'code home'
    target=root/'manual target'
    claude=root/'claude commands'
    opencode=root/'opencode commands'
    # An unrelated non-skill directory is not a destination or a conflict.
    (target/'commands').mkdir(parents=True)
    (target/'commands/keep').write_text('untouched')
    p=run(custom,target,CODEX_HOME=codehome,AGENTBOARD_CLAUDE_COMMANDS=claude,AGENTBOARD_OPENCODE_COMMANDS=opencode)
    assert p.returncode==0,p.stderr
    assert (codehome/'skills/manager/SKILL.md').is_file()
    assert (target/'agentboard-manager').is_symlink()
    assert (target/'commands/keep').read_text()=='untouched'
    assert (claude/'manager.md').is_file() and (opencode/'manager.md').is_file()
    assert (custom/'.hermes/skills/manager/SKILL.md').is_file()
    assert not (custom/'.codex').exists()
    check('custom targets and CODEX_HOME, including spaces, are honored')
print(f'{passed}/{passed} passed (skills installer)')
PY
