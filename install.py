#!/usr/bin/env python3
"""Install agentboard for every supported client.

Builds the agentboard binary, installs the six skills with the `skills` CLI, and registers one global `agentboard` MCP entry per
selected client through that client's own `mcp add` command. The entry is the same
for every seat: the seat's name comes from AGENTBOARD_ACTOR in the environment the
seat is launched with. Role homes and per-seat cards from older installers are never
written or removed; each run only names them as leftovers.
usage: install.py [-h] [--agent AGENT[,AGENT...]] [--force] [--skip-binary]

Unless --skip-binary is given, the agentboard binary is built and installed with
`cargo install --path <repo> --locked` after every preflight check passes and
before any configuration is written, so a failed build changes nothing else.
"""
import argparse
import os
import shutil
import subprocess
import sys
import json

SEATS = ["analyst", "coder", "manager", "reviewer"]
AGENTS = ("codex", "codewhale", "claude", "grok", "agy", "opencode")
CLAUDE_MARK = "agentboardGenerated"
BEGIN = b"# BEGIN AGENTBOARD RULES"
END = b"# END AGENTBOARD RULES"


def split_lines(data):
    """Lines as awk counts them: a last line without LF counts, an empty file has none."""
    parts = data.split(b"\n")
    if parts[-1] == b"":
        parts.pop()
    return parts


def strip_cr(line):
    return line[:-1] if line.endswith(b"\r") else line


def marked_scan(data, begin, end):
    """("none",), ("ok", begin_line, end_line) or ("bad", "3,5").
    A marker is a whole line, ignoring one trailing CR; a line that matches only
    after trimming space and tab counts as a broken marker, never as a marker."""
    nb = ne = bad = fb = fe = 0
    found = []
    for nr, raw in enumerate(split_lines(data), 1):
        line = strip_cr(raw)
        t = line.strip(b" \t")
        if line == begin:
            nb += 1
            fb = fb or nr
            found.append(nr)
        elif line == end:
            ne += 1
            fe = fe or nr
            found.append(nr)
        elif t == begin or t == end:
            bad += 1
            found.append(nr)
    if nb + ne + bad == 0:
        return ("none",)
    if bad or nb != 1 or ne != 1 or fe < fb:
        return ("bad", ",".join(map(str, found)))
    return ("ok", fb, fe)


def rules_scan(data):
    return marked_scan(data, BEGIN, END)


def read(path):
    with open(path, "rb") as f:
        return f.read()


SKILLS_PACKAGE = "skills@1.9.0"
SKILLS_AGENTS = {
    "codex": "codex", "claude": "claude-code", "grok": "universal",
    "codewhale": "universal", "agy": "antigravity", "opencode": "opencode",
}


def find_skills_runner():
    if shutil.which("bunx"):
        return ["bunx", SKILLS_PACKAGE]
    if shutil.which("npx"):
        return ["npx", "-y", SKILLS_PACKAGE]
    return None


def install_skills(runner, repo, agents):
    targets = list(dict.fromkeys(SKILLS_AGENTS[agent] for agent in agents))
    command = [*runner, "add", repo, "-g", "-a", *targets, "-s", "*", "-y"]
    result = subprocess.run(
        command, stdin=subprocess.DEVNULL, capture_output=True, text=True,
        env={**os.environ, "DO_NOT_TRACK": "1"},
    )
    if result.returncode != 0:
        print(result.stdout + result.stderr, file=sys.stderr)
        sys.exit(1)
    print(f"skills: {' '.join(runner)} add {repo} -a {' '.join(targets)}")


def agent_list(value):
    names = value.split(",")
    if any(name not in AGENTS for name in names) or len(set(names)) != len(names):
        raise argparse.ArgumentTypeError(f"expects a comma-list of {','.join(AGENTS)} without duplicates")
    return names



def report_leftovers(home, src):
    """Name what an older installer left behind; never touch it."""
    def repo_link(path):
        return os.path.islink(path) and os.readlink(path).startswith(src + "/")

    found = []
    seats_root = os.path.join(home, ".local", "share", "agentboard", "seats")
    for seat in SEATS:
        for client in AGENTS:
            role_home = os.path.join(seats_root, seat, client)
            if os.path.lexists(role_home):
                found.append(f"{role_home} (role home; holds sessions and credentials, remove it with rm -rf)")
        legacy = os.path.join(home, ".local", "share", "agentboard", "codewhale", seat)
        if os.path.lexists(legacy):
            found.append(f"{legacy} (legacy Codewhale home; remove it with rm -rf)")
        profile = os.path.join(home, ".codex", f"{seat}.config.toml")
        if repo_link(profile):
            found.append(profile)
        card = os.path.join(home, ".claude", f"{seat}-mcp.json")
        if repo_link(card) or (
            os.path.isfile(card) and not os.path.islink(card) and CLAUDE_MARK.encode() in read(card)
        ):
            found.append(card)
    old_launcher = os.path.join(home, ".local", "bin", "start-agentboard-manager")
    if repo_link(old_launcher):
        found.append(old_launcher)
    rules = os.path.join(home, ".codex", "rules", "default.rules")
    if os.path.isfile(rules) and not os.path.islink(rules) and rules_scan(read(rules))[0] == "ok":
        found.append(f"{rules} (agentboard block)")
    for path in found:
        print(f"note: leftover from an older installer, no longer used; delete it by hand: {path}")


GLOBAL_MCP_URL = "http://127.0.0.1:8765/mcp"
GLOBAL_MCP_ADD = {
    "codex": ["codex", "mcp", "add", "agentboard", "--url", GLOBAL_MCP_URL,
              "--bearer-token-env-var", "AGENTBOARD_ACTOR"],
    "claude": ["claude", "mcp", "add-json", "-s", "user", "agentboard", json.dumps(
        {"type": "http", "url": GLOBAL_MCP_URL,
         "headers": {"Authorization": "Bearer ${AGENTBOARD_ACTOR}"}})],
    "grok": ["grok", "mcp", "add", "-s", "user", "--transport", "http", "agentboard", GLOBAL_MCP_URL,
             "-H", "Authorization: Bearer ${AGENTBOARD_ACTOR}"],
    "codewhale": ["codewhale", "mcp", "add", "agentboard", "--url", GLOBAL_MCP_URL,
                  "--bearer-token-env-var", "AGENTBOARD_ACTOR"],
    # agy expands no variable in an entry, so it gets the stdio shim, which inherits the launch env.
    "agy": ["agy", "mcp", "add", "agentboard", os.path.join(os.path.dirname(os.path.abspath(__file__)), "seats/agy/agentboard-stdio")],
    "opencode": ["opencode", "mcp", "add", "--global", "--url", GLOBAL_MCP_URL,
                 "--header", "Authorization=Bearer {env:AGENTBOARD_ACTOR}", "agentboard"],
}


REMOVE_FIRST = {
    "claude": ["claude", "mcp", "remove", "-s", "user", "agentboard"],
    "agy": ["agy", "mcp", "remove", "agentboard"],
}


def run_cli(argv):
    return subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, text=True)


def register_global_mcp(agent):
    """Replace the one global agentboard MCP entry through the client's own command."""
    if not shutil.which(agent):
        print(f"note: {agent} not found on PATH; agentboard MCP entry not registered for it", file=sys.stderr)
        return
    if agent in REMOVE_FIRST:
        # `claude mcp add-json` refuses an existing name; agy's entry is replaced the same way.
        gone = run_cli(REMOVE_FIRST[agent])
        if gone.returncode != 0 and not any(text in gone.stdout + gone.stderr for text in ("No MCP server named", "not found")):
            print(gone.stdout + gone.stderr, file=sys.stderr)
            sys.exit(1)
    added = run_cli(GLOBAL_MCP_ADD[agent])
    if added.returncode != 0:
        print(added.stdout + added.stderr, file=sys.stderr)
        sys.exit(1)
    print(f"registered: agentboard MCP entry for {agent}")



def main():
    parser = argparse.ArgumentParser(
        prog="install.py",
        description="Build and install the agentboard binary, install the agentboard skills with "
        "the skills CLI and register one global agentboard MCP entry per client through its "
        "own `mcp add` command. Nothing else is written to the default client homes.",
    )
    parser.add_argument(
        "--agent",
        type=agent_list,
        action="append",
        metavar="AGENT[,AGENT...]",
        help=f"install only these clients: {','.join(AGENTS)} (default: all, plus the launcher link and systemd unit)",
    )
    parser.add_argument("--force", action="store_true", help="replace existing launcher and unit links")
    parser.add_argument(
        "--skip-binary",
        action="store_true",
        help="do not build and install the agentboard binary (cargo install --path . --locked)",
    )
    args = parser.parse_args()
    if args.agent and len(args.agent) > 1:
        parser.error("argument --agent: may be given only once")
    force = args.force
    selected_agents = args.agent[0] if args.agent else None
    repo = os.path.dirname(os.path.abspath(__file__))
    src = os.path.join(repo, "seats")
    home = os.environ["HOME"]
    selective = selected_agents is not None
    agents = [name for name in ("codex", "claude", "grok", "codewhale", "agy", "opencode") if not selective or name in selected_agents]

    runner = find_skills_runner()
    if runner is None:
        print("neither bunx nor npx found on PATH; install Bun or Node.js to install skills", file=sys.stderr)
        sys.exit(1)

    refuse = False

    def check(p):
        nonlocal refuse
        if os.path.islink(p):
            target = os.readlink(p)
            if not target.startswith(repo + "/") and not force:
                print(f"foreign link: {p} -> {target} (use --force to replace it)", file=sys.stderr)
                refuse = True
        elif os.path.lexists(p):
            if not os.path.isfile(p):
                print(f"not a regular file: {p}", file=sys.stderr)
                refuse = True
            elif not force:
                print(f"exists: {p} (use --force)", file=sys.stderr)
                refuse = True

    if not selective:
        check(f"{home}/.local/bin/agentboard-manager")
        check(f"{home}/.config/systemd/user/agentboard.service")
    if refuse:
        sys.exit(1)
    if not args.skip_binary:
        build = ["cargo", "install", "--path", repo, "--locked"]
        print(f"building: {' '.join(build)}", flush=True)
        try:
            status = subprocess.run(build).returncode
        except FileNotFoundError:
            print("cargo not found; install Rust or rerun with --skip-binary", file=sys.stderr)
            sys.exit(1)
        if status != 0:
            print(f"cargo install failed (exit {status}); nothing else was written", file=sys.stderr)
            sys.exit(1)

    def link(s, d):
        os.makedirs(os.path.dirname(d), exist_ok=True)
        try:
            os.unlink(d)  # rm -f: a real directory must still fail
        except FileNotFoundError:
            pass
        os.symlink(s, d)
        print(f"linked: {d} -> {s}")

    install_skills(runner, repo, agents)
    if not selective:
        link(f"{src}/agentboard-manager", f"{home}/.local/bin/agentboard-manager")
        link(f"{src}/systemd/agentboard.service", f"{home}/.config/systemd/user/agentboard.service")
    for agent in agents:
        register_global_mcp(agent)
    report_leftovers(home, src)
    print("done: agentboard installed; see seats/README.md")


main()
