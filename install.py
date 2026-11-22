#!/usr/bin/env python3
"""Install agentboard for every supported client.

Builds the agentboard binary, links the six skills once into ~/.agents/skills (which
Codex, Grok and Codewhale read), and registers one global `agentboard` MCP entry per
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
AGENTS = ("codex", "codewhale", "claude", "grok", "agy")
CLAUDE_MARK = "agentboardGenerated"
BEGIN = b"# BEGIN AGENTBOARD RULES"
END = b"# END AGENTBOARD RULES"


def split_lines(data):
    """Lines as awk counts them: a last line without LF counts, an empty file has none."""
    parts = data.split(b"\n")
    if parts[-1] == b"":
        parts.pop()
    return parts


def raw_lines(data):
    """The same lines with their terminators, so a rewrite keeps every other byte."""
    return [p + b"\n" for p in data.split(b"\n")[:-1]] + (
        [data.rsplit(b"\n", 1)[-1]] if data and not data.endswith(b"\n") else []
    )


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


def source_ok(data, begin=BEGIN, end=END):
    lines = [strip_cr(x) for x in split_lines(data)]
    markers = sum(1 for x in lines if x.strip(b" \t") in (begin, end))
    return markers == 2 and lines[0] == begin and lines[-1] == end


def read(path):
    with open(path, "rb") as f:
        return f.read()


def skill_plans(home, repo, force, agy=False):
    """Preflight skill links.

    By default the canonical ~/.agents/skills links to the repo, which Codex, Grok and
    Codewhale read. With agy, ~/.gemini/config/skills links to those canonical links,
    because agy does not read ~/.agents/skills.
    """
    skills_src = os.path.join(repo, "skills")
    names = sorted(
        name for name in os.listdir(skills_src)
        if not name.startswith(".") and os.path.isfile(os.path.join(skills_src, name, "SKILL.md"))
    )
    canonical = os.path.join(home, ".agents", "skills")
    base = os.path.join(home, ".gemini", "config", "skills") if agy else canonical
    for path in (os.path.dirname(base), base):
        if os.path.lexists(path) and not os.path.isdir(path):
            raise ValueError(f"invalid skills directory: {path}")
    plans, problems = [], []
    for name in names:
        source = os.path.join(canonical, name) if agy else os.path.join(skills_src, name)
        destination = os.path.join(base, name)
        if os.path.islink(destination) and os.readlink(destination) == source:
            state = "unchanged"
        elif os.path.isdir(destination) and not os.path.islink(destination):
            problems.append(f"real directory at skill destination: {destination}; move it aside")
            continue
        elif os.path.lexists(destination) and not force:
            problems.append(f"exists: {destination} (use --force)")
            continue
        else:
            state = "rewrite" if os.path.lexists(destination) else "new"
        plans.append((source, destination, state))
    if problems:
        raise ValueError("\n".join(problems))
    return plans


def install_skills(plans):
    """Point each canonical skill name at this repo; never follows or removes a directory."""
    for source, destination, state in plans:
        if state == "unchanged":
            continue
        os.makedirs(os.path.dirname(destination), exist_ok=True)
        if os.path.lexists(destination):
            os.unlink(destination)
        os.symlink(source, destination)
        print(f"linked: {destination} -> {source}")


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
        description="Build and install the agentboard binary, link the agentboard skills into "
        "~/.agents/skills and register one global agentboard MCP entry per client through its "
        "own `mcp add` command. Nothing else is written to the default client homes.",
    )
    parser.add_argument(
        "--agent",
        type=agent_list,
        action="append",
        metavar="AGENT[,AGENT...]",
        help=f"install only these clients: {','.join(AGENTS)} (default: all, plus the launcher link and systemd unit)",
    )
    parser.add_argument("--force", action="store_true", help="replace existing skill links and launcher links")
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
    agents = [name for name in ("codex", "claude", "grok", "codewhale", "agy") if not selective or name in selected_agents]

    refuse = False
    try:
        skills = skill_plans(home, repo, force)
        if "agy" in agents:
            agy_skills = skill_plans(home, repo, force, agy=True)
    except (OSError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        skills = agy_skills = []
        refuse = True

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

    install_skills(skills)
    if "agy" in agents:
        install_skills(agy_skills)
    if not selective:
        link(f"{src}/agentboard-manager", f"{home}/.local/bin/agentboard-manager")
        link(f"{src}/systemd/agentboard.service", f"{home}/.config/systemd/user/agentboard.service")
    for agent in agents:
        register_global_mcp(agent)
    report_leftovers(home, src)
    print("done: agentboard installed; see seats/README.md")


main()
