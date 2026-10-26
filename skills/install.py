#!/usr/bin/env python3
"""Canonical repo links in ~/.agents/skills, then agent aliases through that home.
Every skill directory that carries a claude.md or an opencode.md also gets that
CLI's command link, so a seat entry point is one edit in the repo (only opencode
needs one; Claude and Grok invoke the skill directly). Links under a retired name
are pruned when they are exactly the link this installer wrote.
usage: install.py [--force] [canonical-dir]
"""
import os
import shutil
import sys

RETIRED = "manager analyst coder reviewer grok-manager analyzer".split()


def main():
    force = False
    target = ""
    for arg in sys.argv[1:]:
        if arg == "--force":
            force = True
        else:
            target = arg
    home = os.environ["HOME"]
    target = target or f"{home}/.agents/skills"
    if not target.startswith("/"):
        target = f"{os.getcwd()}/{target}"
    src = os.path.dirname(os.path.abspath(__file__))
    codex_home = os.environ.get("CODEX_HOME") or f"{home}/.codex"
    agent_dirs = [
        f"{home}/.claude/skills",
        f"{codex_home}/skills",
        f"{home}/.config/opencode/skills",
        f"{home}/.hermes/skills",
    ]
    claude_commands = os.environ.get("AGENTBOARD_CLAUDE_COMMANDS") or f"{home}/.claude/commands"
    opencode_commands = (
        os.environ.get("AGENTBOARD_OPENCODE_COMMANDS") or f"{home}/.config/opencode/commands"
    )
    names = sorted(
        n
        for n in os.listdir(src)
        if not n.startswith(".") and os.path.isfile(f"{src}/{n}/SKILL.md")
    )
    # A caller may explicitly choose an agent's directory as the canonical home.
    others = [d for d in agent_dirs if d != target]

    # Enumerate the entire write set before touching any parent or destination.
    refuse = False

    def check(p):
        nonlocal refuse
        if os.path.lexists(p) and not force:
            print(f"exists: {p} (use --force)", file=sys.stderr)
            refuse = True

    for name in names:
        check(f"{target}/{name}")
        for d in others:
            check(f"{d}/{name}")
        if os.path.isfile(f"{src}/{name}/claude.md"):
            check(f"{claude_commands}/{name}.md")
        if os.path.isfile(f"{src}/{name}/opencode.md"):
            check(f"{opencode_commands}/{name}.md")
    if refuse:
        sys.exit(1)

    # Retired skill names. A link is removed only when it points exactly where this
    # installer pointed it; a foreign link, a real file or a real directory stays.
    def prune(p, expect):
        if os.path.islink(p) and os.readlink(p) == expect:
            os.unlink(p)
            print(f"pruned: {p}")

    for name in RETIRED:
        if os.path.isdir(f"{src}/{name}"):
            continue
        prune(f"{target}/{name}", f"{src}/{name}")
        for d in others:
            prune(f"{d}/{name}", f"{target}/{name}")
        prune(f"{claude_commands}/{name}.md", f"{target}/{name}/claude.md")
        prune(f"{opencode_commands}/{name}.md", f"{target}/{name}/opencode.md")
    # Claude resolves /<skill> from the skill itself; a command link from before the
    # claude.md files were removed is dead weight.
    for name in names:
        if not os.path.isfile(f"{src}/{name}/claude.md"):
            prune(f"{claude_commands}/{name}.md", f"{target}/{name}/claude.md")

    def link(s, d):
        os.makedirs(os.path.dirname(d), exist_ok=True)
        if os.path.islink(d) or os.path.isfile(d):
            os.unlink(d)
        elif os.path.isdir(d):
            shutil.rmtree(d)
        os.symlink(s, d)
        print(f"linked: {d} -> {s}")

    for name in names:
        link(f"{src}/{name}", f"{target}/{name}")
        for d in others:
            link(f"{target}/{name}", f"{d}/{name}")
        if os.path.isfile(f"{src}/{name}/claude.md"):
            link(f"{target}/{name}/claude.md", f"{claude_commands}/{name}.md")
        if os.path.isfile(f"{src}/{name}/opencode.md"):
            link(f"{target}/{name}/opencode.md", f"{opencode_commands}/{name}.md")


main()
