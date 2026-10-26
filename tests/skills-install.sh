#!/usr/bin/env bash
# Isolated homes: canonical identity, live edits, and atomic refusal.
set -eu
cd "$(dirname "$0")/.."
python3 - <<'PY'
import os, pathlib, subprocess, tempfile
repo=pathlib.Path.cwd()
installer=repo/'skills/install.sh'
passed=0
RETIRED={'manager','analyst','coder','reviewer','grok-manager','analyzer'}

def skills():
    return [d.name for d in sorted((repo/'skills').iterdir()) if (d/'SKILL.md').exists()]

def run(home,*args,**overrides):
    env=dict(os.environ,HOME=str(home))
    for key in ['CODEX_HOME','AGENTBOARD_CLAUDE_COMMANDS','AGENTBOARD_OPENCODE_COMMANDS']:
        env.pop(key,None)
    env.update({k:str(v) for k,v in overrides.items()})
    return subprocess.run([str(installer),*map(str,args)],env=env,capture_output=True,text=True)

def destinations(home,canonical=None,codehome=None,claude=None,opencode=None):
    canonical=canonical or home/'.agents/skills'
    paths={}
    for name in skills():
        paths[canonical/name]=repo/'skills'/name
        for parent in [home/'.claude/skills',(codehome or home/'.codex')/'skills',home/'.config/opencode/skills',home/'.hermes/skills']:
            paths[parent/name]=canonical/name
        for card,root in (('claude',claude or home/'.claude/commands'),('opencode',opencode or home/'.config/opencode/commands')):
            if (repo/'skills'/name/f'{card}.md').exists():
                paths[root/f'{name}.md']=canonical/name/f'{card}.md'
    return paths

def check(name):
    global passed
    passed+=1
    print('PASS',name)

with tempfile.TemporaryDirectory(prefix='agentboard-install-') as root:
    root=pathlib.Path(root);home=root/'fresh'
    p=run(home);assert p.returncode==0,p.stderr
    expected=destinations(home)
    for path,target in expected.items():
        assert path.is_symlink() and os.readlink(path)==str(target),(path,target)
        assert path.resolve()==target.resolve(),path
    assert p.stdout.count('linked: ')==len(expected),p.stdout
    for directory,_,files in os.walk(home/'.hermes/skills',followlinks=True):
        if 'SKILL.md' in files:
            path=pathlib.Path(directory)/'SKILL.md'
            if 'name: agentboard-manager' in path.read_text().split('---',2)[1].splitlines():
                assert path==home/'.hermes/skills/agentboard-manager/SKILL.md',path
    for old in ['manager','agentboard-manager']:
        assert not (home/f'.codex/prompts/{old}.md').exists()
    assert not RETIRED&{p.name.removesuffix('.md') for p in expected},RETIRED
    # One edit, four readers, no reinstall. Remove the probe even on a failure.
    with tempfile.NamedTemporaryFile(dir=home/'.agents/skills/agentboard-team',prefix='.install-probe-',mode='w+') as probe:
        probe.write('one physical copy\n');probe.flush()
        for agent in ['.claude','.codex','.config/opencode','.hermes']:
            alias=home/agent/'skills/agentboard-team'/pathlib.Path(probe.name).name
            assert alias.read_text()=='one physical copy\n'
    check(f'canonical repo links, four indirect aliases per skill, live edit visibility, {len(expected)} destinations')

    p=run(home);assert p.returncode==1 and not p.stdout,p
    for path in expected: assert f'exists: {path} (use --force)' in p.stderr
    check('second run names every canonical, agent, and command conflict without writing')

    conflict=root/'conflict';existing=conflict/'.config/opencode/commands/agentboard-manager.md'
    existing.parent.mkdir(parents=True);existing.write_text('untouched')
    before=existing.stat().st_mtime_ns
    p=run(conflict)
    assert p.returncode==1 and str(existing) in p.stderr and not p.stdout
    assert existing.read_text()=='untouched' and existing.stat().st_mtime_ns==before
    for parent in ['.agents','.claude','.codex','.hermes']:
        assert not (conflict/parent).exists(),parent
    assert not (conflict/'.config/opencode/skills').exists()
    check('a command conflict leaves all earlier parents absent and contents untouched')

    broken=root/'broken';path=broken/'.codex/skills/agentboard-manager'
    path.parent.mkdir(parents=True);path.symlink_to('missing')
    p=run(broken)
    assert p.returncode==1 and str(path) in p.stderr
    assert path.is_symlink() and os.readlink(path)=='missing'
    assert not (broken/'.agents').exists()
    check('dangling agent alias is refused before any canonical directory is created')

    p=run(conflict,'--force');assert p.returncode==0,p.stderr
    for path,target in destinations(conflict).items():
        assert path.is_symlink() and os.readlink(path)==str(target)
    check('force replaces conflicts while retaining canonical indirection')

    custom=root/'custom';target=root/'canonical home';codehome=root/'code home'
    claude=root/'claude commands';opencode=root/'opencode commands'
    (target/'commands').mkdir(parents=True);(target/'commands/keep').write_text('untouched')
    p=run(custom,target,CODEX_HOME=codehome,AGENTBOARD_CLAUDE_COMMANDS=claude,AGENTBOARD_OPENCODE_COMMANDS=opencode)
    assert p.returncode==0,p.stderr
    for path,dest in destinations(custom,target,codehome,claude,opencode).items():
        assert path.is_symlink() and os.readlink(path)==str(dest),(path,dest)
    assert (target/'commands/keep').read_text()=='untouched'
    assert not (custom/'.agents').exists() and not (custom/'.codex').exists()
    check('custom canonical home, CODEX_HOME, and command paths with spaces are honored')

    # Every seat entry point must be one of the installed skills, so a seat can
    # never be launched into a skill that was never linked.
    for seat in ['agentboard-manager','agentboard-analyst','agentboard-coder','agentboard-reviewer']:
        assert seat in skills(),seat
        for card in ('claude','opencode'):
            assert (repo/'skills'/seat/f'{card}.md').exists(),(seat,card)
    assert (repo/'skills/agentboard-team/SKILL.md').exists()
    check('four seat entry points and the shared manual are installed as skills')
    # Retired names: only links this installer wrote are pruned, with no --force.
    old=root/'retired';agents=old/'.agents/skills'
    owned=[agents/'analyzer',old/'.claude/skills/manager',old/'.claude/commands/manager.md',old/'.config/opencode/commands/manager.md']
    owned[0].parent.mkdir(parents=True);owned[0].symlink_to(repo/'skills/analyzer')
    owned[1].parent.mkdir(parents=True);owned[1].symlink_to(agents/'manager')
    owned[2].parent.mkdir(parents=True);owned[2].symlink_to(agents/'manager/claude.md')
    owned[3].parent.mkdir(parents=True);owned[3].symlink_to(agents/'manager/opencode.md')
    foreign=old/'.claude/skills/coder';foreign.symlink_to('/elsewhere')
    realdir=old/'.hermes/skills/reviewer';realdir.mkdir(parents=True);(realdir/'keep').write_text('mine')
    p=run(old);assert p.returncode==0,p.stderr
    for path in owned:
        assert not path.is_symlink() and not path.exists(),path
        assert f'pruned: {path}' in p.stdout,path
    assert foreign.is_symlink() and os.readlink(foreign)=='/elsewhere'
    assert (realdir/'keep').read_text()=='mine' and 'pruned: '+str(realdir) not in p.stdout
    for path,dest in destinations(old).items():
        assert path.is_symlink() and os.readlink(path)==str(dest),path
    check('retired links are pruned only on an exact match; foreign link and real directory stay')
print(f'{passed}/{passed} passed (skills installer)')
PY

# Usage names must match the built CLI and MCP surface before skills ship.
python3 tests/doc-names.py
