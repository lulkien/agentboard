#!/usr/bin/env python3
"""Generate isolated per-role seat homes for every supported client.

Each role gets its own home under ~/.local/share/agentboard/seats/<seat>/<client>,
built from the repo cards plus validated copies of the user's base settings and
credentials. The default client homes (~/.codex, ~/.claude, ~/.grok, ~/.codewhale)
are read, and each selected client gets one global agentboard MCP entry written through
its own `mcp add` command; files an older installer left there are only named.
The five skills are linked once into ~/.agents/skills, which Codex, Grok and
Codewhale read from any role home; Claude role homes link them into skills/.
usage: install.py [-h] [--agent AGENT[,AGENT...]] [--force] [--skip-binary]

Unless --skip-binary is given, the agentboard binary is built and installed with
`cargo install --path <repo> --locked` after every preflight check passes and
before any configuration is written, so a failed build changes nothing else.
"""
import argparse
import json
import hashlib
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import tomllib

SEATS = ["analyst", "coder", "manager", "reviewer"]
AGENTS = ("codex", "codewhale", "claude", "grok")
CODEX_HOME_ROOT = ".local/share/agentboard/seats"
CLAUDE_HOME_ROOT = ".local/share/agentboard/seats"
CODEX_SEAT_MARK = b"agentboard Codex seat home v1\n"
CLAUDE_SEAT_MARK = b"agentboard Claude seat home v1\n"
CLAUDE_MARK = "agentboardGenerated"
CLAUDE_MARK_TEXT = "written by agentboard install.py from seats/claude/<seat>-mcp.json and ~/.claude.json mcpServers; rerun it to refresh, --force overwrites edits"
BEGIN = b"# BEGIN AGENTBOARD RULES"
END = b"# END AGENTBOARD RULES"
CODEWHALE_MARK = b"# agentboardGenerated"
GROK_BEGIN = b"# BEGIN AGENTBOARD MCP"
GROK_END = b"# END AGENTBOARD MCP"
GROK_HOME_ROOT = ".local/share/agentboard/seats"


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


def codex_seat_home_plans(home, src, force):
    """Preflight role homes, reporting missing base auth and stale snapshots."""
    base = os.path.join(home, ".codex")
    if os.path.lexists(base) and (os.path.islink(base) or not os.path.isdir(base)):
        raise ValueError(f"invalid Codex base home: {base}")
    inputs = {}
    for name, parser in (("config.toml", "toml"), ("auth.json", "json")):
        path = os.path.join(base, name)
        if not os.path.lexists(path):
            inputs[name] = None
            continue
        if os.path.islink(path) or not os.path.isfile(path):
            raise ValueError(f"invalid Codex base file: {path}")
        payload = read(path)
        try:
            if parser == "toml":
                tomllib.loads(payload.decode("utf-8"))
            else:
                parsed = json.loads(payload)
                if not isinstance(parsed, dict):
                    raise ValueError("top level must be an object")
        except (UnicodeDecodeError, json.JSONDecodeError, tomllib.TOMLDecodeError, ValueError) as exc:
            raise ValueError(f"invalid {parser.upper()} in {path}: {exc}") from exc
        if name == "config.toml":
            # The global agentboard entry authenticates from AGENTBOARD_ACTOR, which role
            # homes do not set; the role overlay supplies its own entry.
            cleaned = remove_agentboard_table(payload)
            if cleaned is not None:
                payload = cleaned
        inputs[name] = payload

    missing_auth = inputs["auth.json"] is None
    plans = []
    stale_auth = []
    parent = os.path.join(home, CODEX_HOME_ROOT)
    for seat in SEATS:
        destination = os.path.join(parent, seat, "codex")
        # Reject symlinks in any existing parent component before creating homes.
        current = home
        for component in (".local", "share", "agentboard", "seats", seat, "codex"):
            current = os.path.join(current, component)
            if os.path.islink(current):
                raise ValueError(f"unsafe Codex seat home path: {current}")
            if os.path.lexists(current) and not os.path.isdir(current):
                raise ValueError(f"invalid Codex seat home path: {current}")
        marker = os.path.join(destination, ".agentboard-seat")
        if os.path.lexists(destination):
            if not os.path.isfile(marker) or os.path.islink(marker) or read(marker) != CODEX_SEAT_MARK:
                raise ValueError(f"unmanaged Codex seat home: {destination}; move it aside before installing")
        files = {".agentboard-seat": CODEX_SEAT_MARK}
        for name, payload in inputs.items():
            if payload is not None:
                files[name] = payload
        profile_path = os.path.join(src, "codex", f"{seat}.config.toml")
        profile = read(profile_path)
        try:
            profile_data = tomllib.loads(profile.decode("utf-8"))
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
            raise ValueError(f"invalid TOML in Codex role profile {profile_path}: {exc}") from exc
        policy = profile_data.get("approval_policy")
        if policy not in ("never", "on-request") or not (
            isinstance(profile_data.get("sandbox_mode"), str)
            or isinstance(profile_data.get("default_permissions"), str)
        ):
            raise ValueError(f"Codex role profile lacks an explicit policy: {profile_path}")
        # Codex validates the profile file on its own when it writes to it (folder
        # trust, for one): a server table with neither command nor url is an
        # "invalid transport" there, even when config.toml defines that server.
        servers = profile_data.get("mcp_servers", {})
        bare = sorted(name for name, entry in servers.items() if not ({"command", "url"} & set(entry)))
        if bare:
            raise ValueError(f"Codex role profile has MCP servers without command or url: {profile_path}: {', '.join(bare)}")
        files[f"{seat}.config.toml"] = profile
        cleanup = []
        for old_seat in SEATS:
            old_profile = os.path.join(destination, f"{old_seat}.config.toml")
            if os.path.islink(old_profile):
                target = os.readlink(old_profile)
                if not os.path.isabs(target):
                    target = os.path.join(destination, target)
                expected = os.path.abspath(os.path.join(src, "codex", f"{old_seat}.config.toml"))
                if os.path.abspath(target) != expected:
                    raise ValueError(f"unsafe Codex role profile link: {old_profile}")
                cleanup.append(old_profile)
        rules_dir = os.path.join(destination, "rules")
        if os.path.lexists(rules_dir):
            if os.path.islink(rules_dir) or not os.path.isdir(rules_dir):
                raise ValueError(f"unsafe Codex role rules path: {rules_dir}")
            default_rules = os.path.join(rules_dir, "default.rules")
            if os.path.lexists(default_rules):
                if os.path.islink(default_rules) or not os.path.isfile(default_rules):
                    raise ValueError(f"unsafe Codex role rules file: {default_rules}")
                current_rules = read(default_rules)
                scan = rules_scan(current_rules)
                if scan[0] == "bad" or (scan[0] == "none" and current_rules.strip()):
                    raise ValueError(f"unmanaged rules in restricted Codex home: {default_rules}")
                cleanup.append(default_rules)
            old_rules = os.path.join(rules_dir, "agentboard.rules")
            if os.path.lexists(old_rules):
                if not os.path.islink(old_rules):
                    raise ValueError(f"unmanaged rules in restricted Codex home: {old_rules}")
                target = os.readlink(old_rules)
                if not os.path.isabs(target):
                    target = os.path.join(rules_dir, target)
                expected = os.path.abspath(os.path.join(src, "codex", "rules", "agentboard.rules"))
                if os.path.abspath(target) != expected:
                    raise ValueError(f"unsafe Codex role rules link: {old_rules}")
                cleanup.append(old_rules)
            unknown_rules = [
                name for name in os.listdir(rules_dir)
                if name not in {"default.rules", "agentboard.rules"} and name.endswith(".rules")
            ]
            if unknown_rules:
                raise ValueError(f"unmanaged rules in restricted Codex home: {rules_dir}")
        for name, payload in files.items():
            path = os.path.join(destination, name)
            if os.path.islink(path) and path in cleanup:
                continue
            if os.path.lexists(path) and (os.path.islink(path) or not os.path.isfile(path)):
                raise ValueError(f"invalid Codex seat file: {path}")
            if os.path.isfile(path) and not force and (read(path) != payload or os.stat(path).st_mode & 0o777 != 0o600):
                raise ValueError(f"Codex seat home differs from installer input: {path} (use --force)")
        auth_path = os.path.join(destination, "auth.json")
        if missing_auth and os.path.lexists(auth_path):
            if os.path.islink(auth_path) or not os.path.isfile(auth_path):
                raise ValueError(f"invalid Codex seat auth file: {auth_path}")
            stale_auth.append(auth_path)
        plans.append((destination, files, cleanup))
    return plans, missing_auth, stale_auth


def cleanup_codex_seat_homes(plans, stale_auth=()):
    """Remove legacy shared rules and profiles, and explicitly revoked auth."""
    for destination, _files, cleanup in plans:
        for path in cleanup:
            if os.path.basename(path) == "default.rules" or os.path.islink(path):
                os.unlink(path)
        rules_dir = os.path.join(destination, "rules")
        if os.path.isdir(rules_dir) and not os.listdir(rules_dir):
            os.rmdir(rules_dir)
    for path in stale_auth:
        os.unlink(path)
        print(f"removed stale Codex auth snapshot: {path}")


def install_codex_seat_homes(plans):
    """Write the already-preflighted Codex role homes without following links."""
    for destination, files, cleanup in plans:
        os.makedirs(destination, mode=0o700, exist_ok=True)
        os.chmod(destination, 0o700)
        for name, payload in files.items():
            path = os.path.join(destination, name)
            if os.path.isfile(path) and not os.path.islink(path) and read(path) == payload and os.stat(path).st_mode & 0o777 == 0o600:
                continue
            tmp = f"{path}.agentboard.{os.getpid()}"
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                os.fchmod(fd, 0o600)
                with os.fdopen(fd, "wb") as stream:
                    stream.write(payload)
                os.replace(tmp, path)
            finally:
                if os.path.lexists(tmp):
                    os.unlink(tmp)
        cleanup_codex_seat_homes([(destination, files, cleanup)])
        print(f"generated: {destination} (Codex role home)")


def skill_plans(home, repo, force):
    """Preflight the canonical ~/.agents/skills links.

    Codex, Grok and Codewhale read HOME/.agents/skills even when their config
    home is a role home, so these links are all they need; Claude role homes link
    each seat skill through them.
    """
    skills_src = os.path.join(repo, "skills")
    names = sorted(
        name for name in os.listdir(skills_src)
        if not name.startswith(".") and os.path.isfile(os.path.join(skills_src, name, "SKILL.md"))
    )
    base = os.path.join(home, ".agents", "skills")
    for path in (os.path.dirname(base), base):
        if os.path.lexists(path) and not os.path.isdir(path):
            raise ValueError(f"invalid skills directory: {path}")
    plans, problems = [], []
    for name in names:
        source = os.path.join(skills_src, name)
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


def claude_base_json(path, optional=False):
    """Read one Claude user-scope JSON file without following unsafe inputs."""
    if not os.path.lexists(path):
        if optional:
            return None
        return {}
    if os.path.islink(path) or not os.path.isfile(path):
        raise ValueError(f"invalid Claude base file: {path}")
    try:
        with open(path, "rb") as stream:
            value = json.load(stream)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise ValueError(f"invalid JSON in {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise ValueError(f"invalid JSON in {path}: top level must be an object")
    return value


def claude_home_plans(home, src, force):
    repo = os.path.dirname(src)
    """Preflight all role homes before any Claude publication."""
    base = os.path.join(home, ".claude")
    if os.path.islink(base) or (os.path.lexists(base) and not os.path.isdir(base)):
        raise ValueError(f"invalid Claude base directory: {base}")
    settings = claude_base_json(os.path.join(base, "settings.json"))
    global_config = claude_base_json(os.path.join(home, ".claude.json"))
    credential_path = os.path.join(base, ".credentials.json")
    credentials = None
    if os.path.lexists(credential_path):
        if os.path.islink(credential_path) or not os.path.isfile(credential_path):
            raise ValueError(f"invalid Claude base file: {credential_path}")
        claude_base_json(credential_path)
        credentials = read(credential_path)

    user_servers = global_config.get("mcpServers", {})
    if not isinstance(user_servers, dict):
        raise ValueError(f"mcpServers in {os.path.join(home, '.claude.json')} is not an object")
    plans = []
    canonical_skills = os.path.join(home, ".agents", "skills")
    for seat in SEATS:
        destination = os.path.join(home, CLAUDE_HOME_ROOT, seat, "claude")
        current = home
        for component in (".local", "share", "agentboard", "seats", seat, "claude"):
            current = os.path.join(current, component)
            if os.path.islink(current):
                raise ValueError(f"unsafe Claude seat home path: {current}")
            if os.path.lexists(current) and not os.path.isdir(current):
                raise ValueError(f"invalid Claude seat home path: {current}")
        marker = os.path.join(destination, ".agentboard-seat")
        if os.path.lexists(destination) and (
            os.path.islink(marker) or not os.path.isfile(marker) or read(marker) != CLAUDE_SEAT_MARK
        ):
            raise ValueError(f"unmanaged Claude seat home: {destination}; move it aside before installing")

        role_settings = dict(settings)
        permissions = role_settings.get("permissions", {})
        if not isinstance(permissions, dict):
            raise ValueError(f"permissions in {os.path.join(base, 'settings.json')} is not an object")
        permissions = dict(permissions)
        if seat in ("analyst", "reviewer"):
            denied = permissions.get("deny", [])
            if not isinstance(denied, list) or any(not isinstance(item, str) for item in denied):
                raise ValueError(f"permissions.deny in {os.path.join(base, 'settings.json')} must be a string array")
            permissions["deny"] = list(dict.fromkeys([*denied, "Edit", "Write", "NotebookEdit"]))
        role_settings["permissions"] = permissions
        role_config = dict(global_config)
        # Claude stores project trust, approvals and project MCP state under this
        # map. A relocated user config must not promote it to global role state.
        role_config.pop("projects", None)
        role_config["mcpServers"] = {**user_servers}
        card_path = os.path.join(src, "claude", f"{seat}-mcp.json")
        with open(card_path, encoding="utf-8") as stream:
            card = json.load(stream)
        merged_servers = {**user_servers, **card.get("mcpServers", {})}
        mcp_config = {CLAUDE_MARK: CLAUDE_MARK_TEXT, "mcpServers": merged_servers}
        files = {
            ".agentboard-seat": CLAUDE_SEAT_MARK,
            "settings.json": (json.dumps(role_settings, indent=2) + "\n").encode(),
            ".claude.json": (json.dumps(role_config, indent=2) + "\n").encode(),
            "mcp.json": (json.dumps(mcp_config, indent=2) + "\n").encode(),
        }
        if credentials is not None:
            files[".credentials.json"] = credentials
            files[".agentboard-credentials.sha256"] = (hashlib.sha256(credentials).hexdigest() + "\n").encode()
        stale_credentials = []
        credential_warnings = []
        role_credentials = os.path.join(destination, ".credentials.json")
        role_credential_mark = os.path.join(destination, ".agentboard-credentials.sha256")
        copied_snapshot = False
        if os.path.lexists(role_credentials):
            if os.path.islink(role_credentials) or not os.path.isfile(role_credentials):
                raise ValueError(f"unsafe Claude seat file: {role_credentials}")
            if os.path.lexists(role_credential_mark):
                if os.path.islink(role_credential_mark) or not os.path.isfile(role_credential_mark):
                    raise ValueError(f"unsafe Claude seat file: {role_credential_mark}")
                recorded_hash = read(role_credential_mark).decode("ascii", errors="ignore").strip()
                copied_snapshot = (
                    len(recorded_hash) == 64
                    and recorded_hash == hashlib.sha256(read(role_credentials)).hexdigest()
                )
            if credentials is None:
                if force and copied_snapshot:
                    stale_credentials.extend((role_credentials, role_credential_mark))
                elif not copied_snapshot:
                    credential_warnings.append(role_credentials)
            elif not copied_snapshot:
                # A role login without a matching installer snapshot marker is
                # role-owned. Never overwrite it with the base account.
                files.pop(".credentials.json", None)
                files.pop(".agentboard-credentials.sha256", None)
                credential_warnings.append(role_credentials)
        elif os.path.lexists(role_credential_mark):
            if os.path.islink(role_credential_mark) or not os.path.isfile(role_credential_mark):
                raise ValueError(f"unsafe Claude seat file: {role_credential_mark}")
            if force:
                stale_credentials.append(role_credential_mark)
        skills_home = os.path.join(destination, "skills")
        if os.path.islink(skills_home) or (os.path.lexists(skills_home) and not os.path.isdir(skills_home)):
            raise ValueError(f"unsafe Claude skills directory: {skills_home}")
        for skill in (f"agentboard-{seat}", "agentboard-team"):
            # The canonical link itself is planned and checked by skill_plans.
            source = os.path.join(canonical_skills, skill)
            if not os.path.isfile(os.path.join(repo, "skills", skill, "SKILL.md")):
                raise ValueError(f"repo skill missing: {os.path.join(repo, 'skills', skill)}")
            link_path = os.path.join(destination, "skills", skill)
            if os.path.isdir(link_path) and not os.path.islink(link_path):
                raise ValueError(f"unsafe Claude skill destination: {link_path}")
            if os.path.lexists(link_path) and (
                not os.path.islink(link_path) or os.readlink(link_path) != source
            ) and not force:
                raise ValueError(f"unmanaged Claude skill destination: {link_path}; use --force")
        states = {}
        for name, payload in files.items():
            path = os.path.join(destination, name)
            if os.path.islink(path) or (os.path.lexists(path) and not os.path.isfile(path)):
                raise ValueError(f"unsafe Claude seat file: {path}")
            if os.path.lexists(path) and not force:
                if not os.path.isfile(marker) or read(marker) != CLAUDE_SEAT_MARK:
                    raise ValueError(f"unmanaged Claude seat file: {path}; use --force")
                if read(path) == payload and stat.S_IMODE(os.stat(path).st_mode) == 0o600:
                    states[path] = "unchanged"
                else:
                    print(f"note: {path} differs from Claude base settings; --force rewrites it", file=sys.stderr)
                    states[path] = "differs"
            else:
                states[path] = "rewrite" if os.path.lexists(path) else "new"

        plans.append((destination, files, states, seat, stale_credentials, credential_warnings))
    return plans


def install_claude_seat_homes(plans, home):
    canonical_skills = os.path.join(home, ".agents", "skills")
    warnings_to_print = []
    for destination, files, states, seat, stale_credentials, role_warnings in plans:
        os.makedirs(destination, mode=0o700, exist_ok=True)
        os.chmod(destination, 0o700)
        for name, payload in files.items():
            path = os.path.join(destination, name)
            if states[path] in ("unchanged", "differs"):
                continue
            tmp = f"{path}.agentboard.{os.getpid()}"
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                os.fchmod(fd, 0o600)
                with os.fdopen(fd, "wb") as stream:
                    stream.write(payload)
                os.replace(tmp, path)
            finally:
                if os.path.lexists(tmp):
                    os.unlink(tmp)
        os.chmod(os.path.join(destination, ".agentboard-seat"), 0o600)
        skills_dir = os.path.join(destination, "skills")
        os.makedirs(skills_dir, mode=0o700, exist_ok=True)
        os.chmod(skills_dir, 0o700)
        for skill in (f"agentboard-{seat}", "agentboard-team"):
            source = os.path.join(canonical_skills, skill)
            target = os.path.join(skills_dir, skill)
            if os.path.islink(target) and os.readlink(target) == source:
                continue
            if os.path.lexists(target):
                os.unlink(target)
            os.symlink(source, target)
        os.chmod(destination, 0o700)
        for path in stale_credentials:
            if not os.path.lexists(path):
                continue
            os.unlink(path)
            if path.endswith("/.credentials.json"):
                print(f"removed stale Claude credentials: {path}")
        warnings_to_print.extend(role_warnings)
        verb = "generated" if any(state in ("new", "rewrite") for state in states.values()) else "unchanged"
        print(f"{verb}: {destination} (Claude role home)")
    for path in warnings_to_print:
        print(
            f"warning: preserving Claude role credentials with unknown ownership; "
            f"inspect and remove manually if stale: {path}",
            file=sys.stderr,
        )


def block_bytes(source, begin, end, nl):
    return b"".join(strip_cr(x) + nl for x in split_lines(source))


def marked_candidate(before, exists, state, source, begin, end):
    if state[0] == "replace-unmarked":
        before = state[1]
        state = ("none",)
    first = before.find(b"\n")
    nl = b"\r\n" if first > 0 and before[first - 1 : first] == b"\r" else b"\n"
    block = block_bytes(source, begin, end, nl)
    if not exists:
        return block
    if state[0] == "ok":
        raw = raw_lines(before)
        return b"".join(raw[: state[1] - 1]) + block + b"".join(raw[state[2] :])
    sep = nl if before and not before.endswith(b"\n") else b""
    return before + sep + block


def remove_agentboard_table(data):
    """Remove safely identifiable agentboard table sections, preserving other bytes."""
    raw = raw_lines(data)
    headers = []
    target = []
    bare_path = rb"mcp_servers(?:\.[A-Za-z0-9_-]+)+"
    exact = re.compile(rb"^[ \t]*\[(" + bare_path + rb")\][ \t]*(?:#.*)?$")
    key = rb'(?:[A-Za-z0-9_-]+|"(?:[^"\\]|\\.)*"|\'[^\']*\')'
    key_path = key + rb"(?:[ \t]*\.[ \t]*" + key + rb")*"
    any_header = re.compile(
        rb"^[ \t]*(?:\[" + key_path + rb"\]|\[\[" + key_path + rb"\]\])"
        rb"[ \t]*(?:#.*)?$"
    )
    for index, line in enumerate(raw):
        stripped = strip_cr(line[:-1] if line.endswith(b"\n") else line)
        match = exact.fullmatch(stripped)
        if match:
            path = match.group(1).split(b".")
            headers.append(index)
            if path[:2] == [b"mcp_servers", b"agentboard"]:
                target.append(index)
        elif any_header.fullmatch(stripped):
            headers.append(index)
    if not target:
        return None
    remove = set()
    for start in target:
        end = next((index for index in headers if index > start), len(raw))
        if end == len(raw):  # last table: also drop the blank separator before it
            while start > 0 and not strip_cr(raw[start - 1]).strip():
                start -= 1
        while end > start + 1:
            trailing = strip_cr(raw[end - 1]).strip()
            if trailing and not trailing.startswith(b"#"):
                break
            end -= 1
        remove.update(range(start, end))
    return b"".join(line for index, line in enumerate(raw) if index not in remove)


def grok_preflight(path, source, force, strip_unmarked=False):
    if os.path.islink(path):
        print(f"{path} is a symlink; the installer edits only a regular file", file=sys.stderr)
        return False, None
    if os.path.exists(path) and not os.path.isfile(path):
        print(f"{path} is not a regular file; the installer edits only a regular file", file=sys.stderr)
        return False, None
    if not source_ok(source, GROK_BEGIN, GROK_END):
        print(f"bad Grok MCP source (must start/end with markers): {path}", file=sys.stderr)
        return False, None
    try:
        tomllib.loads(source.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        print(f"invalid TOML in Grok MCP source: {exc}", file=sys.stderr)
        return False, None
    if not os.path.isfile(path):
        return True, ("missing",)
    before = read(path)
    scan = marked_scan(before, GROK_BEGIN, GROK_END)
    if scan[0] == "bad":
        print(f"malformed agentboard block in {path} (marker lines {scan[1]}); fix it by hand", file=sys.stderr)
        return False, None
    try:
        tomllib.loads(before.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        print(f"invalid TOML in {path}: {exc}", file=sys.stderr)
        return False, None
    outside = before
    if scan[0] == "ok":
        raw = raw_lines(before)
        outside = b"".join(raw[: scan[1] - 1]) + b"".join(raw[scan[2] :])
    try:
        parsed = tomllib.loads(outside.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
        print(f"invalid TOML in {path} outside the marked agentboard block: {exc}", file=sys.stderr)
        return False, None
    servers = parsed.get("mcp_servers", {})
    if "mcp_servers" in parsed and not isinstance(servers, dict):
        print(f"invalid mcp_servers table in {path}; review it by hand", file=sys.stderr)
        return False, None
    if isinstance(servers, dict) and "agentboard" in servers:
        if not (force or strip_unmarked):
            print(
                f"unmarked [mcp_servers.agentboard] entry in {path}; review and remove or convert it by hand",
                file=sys.stderr,
            )
            return False, None
        cleaned = remove_agentboard_table(outside)
        if cleaned is None:
            print(f"cannot safely locate unmarked [mcp_servers.agentboard] entry in {path}", file=sys.stderr)
            return False, None
        try:
            tomllib.loads(cleaned.decode("utf-8"))
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
            print(f"cannot safely remove unmarked Grok entry from {path}: {exc}", file=sys.stderr)
            return False, None
        return True, ("replace-unmarked", cleaned)
    return True, scan


def grok_managed_block_matches(data, source):
    """Compare only the marked Grok block, ignoring CRLF and final newline shape."""
    state = marked_scan(data, GROK_BEGIN, GROK_END)
    if state[0] != "ok":
        return False
    lines = split_lines(data)
    existing = b"\n".join(lines[state[1] - 1 : state[2]]).replace(b"\r", b"")
    expected = source.replace(b"\r", b"").rstrip(b"\n")
    return existing == expected


def grok_seat_home_plans(home, src, force):
    """Build isolated Grok configs from safe user settings and credentials."""
    base = os.path.join(home, ".grok")
    if os.path.lexists(base) and (os.path.islink(base) or not os.path.isdir(base)):
        raise ValueError(f"invalid Grok base home: {base}")
    base_config = os.path.join(base, "config.toml")
    if os.path.lexists(base_config) and (os.path.islink(base_config) or not os.path.isfile(base_config)):
        raise ValueError(f"invalid Grok base config: {base_config}")
    settings = b""
    if os.path.isfile(base_config):
        settings = read(base_config)
        try:
            tomllib.loads(settings.decode("utf-8"))
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
            raise ValueError(f"invalid TOML in {base_config}: {exc}") from exc
    source = read(os.path.join(src, "grok/agentboard-mcp.toml"))
    if not source_ok(source, GROK_BEGIN, GROK_END):
        raise ValueError("bad Grok MCP source")
    plans = []
    for seat in SEATS:
        dest = os.path.join(home, GROK_HOME_ROOT, seat, "grok")
        current = home
        for component in (".local", "share", "agentboard", "seats", seat, "grok"):
            current = os.path.join(current, component)
            if os.path.islink(current) or (os.path.lexists(current) and not os.path.isdir(current)):
                raise ValueError(f"unsafe Grok seat home path: {current}")
        files = {}
        role_source = source.replace(b"${AGENTBOARD_SEAT:-manager}", seat.encode())
        base_ok, base_state = grok_preflight(base_config, role_source, force, strip_unmarked=True)
        if not base_ok:
            raise ValueError(f"unsafe Grok base config: {base_config}")
        role_config = os.path.join(dest, "config.toml")
        if os.path.lexists(role_config) and (os.path.islink(role_config) or not os.path.isfile(role_config)):
            raise ValueError(f"invalid Grok seat config: {role_config}")
        if os.path.isfile(role_config):
            role_before = read(role_config)
            role_ok, role_state = grok_preflight(role_config, role_source, force)
            if not role_ok:
                raise ValueError(f"unsafe Grok role config: {role_config}")
            if role_state[0] == "ok" and not force:
                candidate = role_before
                try:
                    actor = tomllib.loads(candidate.decode("utf-8"))["mcp_servers"]["agentboard"]["url"]
                except (UnicodeDecodeError, tomllib.TOMLDecodeError, KeyError, TypeError):
                    raise ValueError(f"invalid agentboard URL in {role_config}")
                if actor != f"http://127.0.0.1:8765/mcp/{seat}":
                    raise ValueError(f"Grok role actor differs in {role_config} (use --force)")
                if not grok_managed_block_matches(role_before, role_source):
                    raise ValueError(f"Grok managed agentboard block differs in {role_config} (use --force)")
            else:
                candidate = marked_candidate(role_before, True, role_state, role_source, GROK_BEGIN, GROK_END)
        else:
            candidate = marked_candidate(settings, bool(settings), base_state, role_source, GROK_BEGIN, GROK_END)
        tomllib.loads(candidate.decode("utf-8"))
        files["config.toml"] = candidate
        manifest_path = os.path.join(dest, ".agentboard-grok-state.json")
        manifest = {}
        if os.path.lexists(manifest_path):
            if os.path.islink(manifest_path) or not os.path.isfile(manifest_path):
                raise ValueError(f"invalid Grok ownership state: {manifest_path}")
            try:
                manifest = json.loads(read(manifest_path))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise ValueError(f"invalid Grok ownership state: {manifest_path}") from exc
            if not isinstance(manifest, dict) or any(k not in ("auth.json", "mcp_credentials.json") or not re.fullmatch(r"[0-9a-f]{64}", v or "") for k, v in manifest.items()):
                raise ValueError(f"invalid Grok ownership state: {manifest_path}")
        next_manifest = {}
        for name in ("auth.json", "mcp_credentials.json"):
            path = os.path.join(base, name)
            payload = None
            if os.path.lexists(path):
                if os.path.islink(path) or not os.path.isfile(path):
                    raise ValueError(f"invalid Grok credential file: {path}")
                payload = read(path)
                try:
                    value = json.loads(payload)
                    if not isinstance(value, dict):
                        raise ValueError(f"invalid JSON in {path}: top level must be an object")
                except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                    raise ValueError(f"invalid JSON in {path}: {exc}") from exc
            role_path = os.path.join(dest, name)
            prior_hash = manifest.get(name)
            if os.path.lexists(role_path) and (os.path.islink(role_path) or not os.path.isfile(role_path)):
                raise ValueError(f"invalid Grok role credential: {role_path}")
            prior_owned = bool(os.path.isfile(role_path) and prior_hash and hashlib.sha256(read(role_path)).hexdigest() == prior_hash)
            if os.path.isfile(role_path) and not prior_owned:
                print(f"warning: preserving independent Grok role credential: {role_path}", file=sys.stderr)
            elif prior_owned and payload is None:
                if force:
                    files[name] = None
                else:
                    next_manifest[name] = prior_hash
            elif prior_owned and payload is not None and not force and hashlib.sha256(payload).hexdigest() != prior_hash:
                next_manifest[name] = prior_hash
            elif payload is not None:
                files[name] = payload
                next_manifest[name] = hashlib.sha256(payload).hexdigest()
        files[".agentboard-grok-state.json"] = (json.dumps(next_manifest, sort_keys=True, separators=(",", ":")) + "\n").encode()
        marker = os.path.join(dest, ".agentboard-seat")
        if os.path.lexists(dest) and (not os.path.isfile(marker) or os.path.islink(marker) or read(marker) != b"agentboard Grok seat home v1\n"):
            raise ValueError(f"unmanaged Grok seat home: {dest}")
        files[".agentboard-seat"] = b"agentboard Grok seat home v1\n"
        for name, payload in files.items():
            path = os.path.join(dest, name)
            if payload is None:
                continue
            if os.path.lexists(path) and (os.path.islink(path) or not os.path.isfile(path)):
                raise ValueError(f"invalid Grok seat file: {path}")
            if os.path.isfile(path) and not force and name not in ("auth.json", "mcp_credentials.json", ".agentboard-grok-state.json") and read(path) != payload:
                raise ValueError(f"Grok seat home differs from installer input: {path} (use --force)")
        plans.append((dest, files))
    return plans


def install_grok_seat_homes(plans):
    for dest, files in plans:
        os.makedirs(dest, mode=0o700, exist_ok=True)
        os.chmod(dest, 0o700)
        for name, payload in files.items():
            path = os.path.join(dest, name)
            if payload is None:
                if os.path.isfile(path) and not os.path.islink(path):
                    os.unlink(path)
                continue
            if os.path.isfile(path) and not os.path.islink(path) and read(path) == payload and os.stat(path).st_mode & 0o777 == 0o600:
                continue
            tmp = f"{path}.agentboard.{os.getpid()}"
            fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                os.fchmod(fd, 0o600)
                with os.fdopen(fd, "wb") as stream:
                    stream.write(payload)
                os.replace(tmp, path)
            finally:
                if os.path.lexists(tmp): os.unlink(tmp)
        for name in ("auth.json", "mcp_credentials.json"):
            path = os.path.join(dest, name)
            if os.path.isfile(path) and not os.path.islink(path):
                os.chmod(path, 0o600)
        print(f"generated: {dest} (Grok role home)")


def codewhale_home_files(src, home, seat):
    """(config.toml bytes, mcp.json bytes) for one seat, based on user config when present."""
    with open(f"{src}/claude/{seat}-mcp.json", encoding="utf-8") as f:
        url = json.load(f)["mcpServers"]["agentboard"]["url"]
    mcp_path = f"{os.environ['HOME']}/.codewhale/mcp.json"
    try:
        with open(mcp_path, encoding="utf-8") as f:
            mcp = json.load(f)
    except FileNotFoundError:
        mcp = {}
    except (json.JSONDecodeError, UnicodeDecodeError) as exc:
        raise ValueError(f"invalid JSON in {mcp_path}: {exc}") from exc
    if not isinstance(mcp, dict):
        raise ValueError(f"invalid JSON in {mcp_path}: top level must be an object")
    servers = mcp.setdefault("servers", {})
    if not isinstance(servers, dict):
        raise ValueError(f"servers in {mcp_path} is not an object")
    servers["agentboard"] = {"url": url}

    config_path = f"{os.environ['HOME']}/.codewhale/config.toml"
    config = {}
    try:
        with open(config_path, "rb") as f:
            config = tomllib.load(f)
    except FileNotFoundError:
        pass
    except (tomllib.TOMLDecodeError, UnicodeDecodeError) as exc:
        raise ValueError(f"invalid TOML in {config_path}: {exc}") from exc
    # Auth belongs in Codewhale's credential store, not a generated seat config.
    def without_credentials(value):
        if isinstance(value, dict):
            return {k: without_credentials(v) for k, v in value.items()
                    if not re.search(r"(api.?key|token|secret|credential|password)", k, re.I)}
        if isinstance(value, list):
            return [without_credentials(item) for item in value]
        return value

    config = without_credentials(config)
    selected_provider = config.get("provider")
    if selected_provider is not None and (
        not isinstance(selected_provider, str)
        or not re.fullmatch(r"[A-Za-z0-9_-]+", selected_provider)
    ):
        raise ValueError("invalid provider in ~/.codewhale/config.toml")
    # Execution and permission posture belongs to each seat template. Do not let
    # a user-wide policy quietly widen a manager/coder or constrain a reviewer.
    for key in ("approval_policy", "sandbox_mode", "sandbox_network_access"):
        config.pop(key, None)
    with open(f"{src}/codewhale/{seat}.config.toml", encoding="utf-8") as f:
        seat_config = tomllib.loads(f.read().replace("@HOME@", home))
    with open(f"{src}/codewhale/status-hooks.toml", "rb") as f:
        hooks = tomllib.load(f)

    def merge(dst, overlay):
        for key, value in overlay.items():
            if isinstance(value, dict) and isinstance(dst.get(key), dict):
                merge(dst[key], value)
            else:
                dst[key] = value

    merge(config, seat_config)
    merge(config, hooks)
    config_bytes = toml_bytes(config)
    mcp_bytes = (json.dumps(mcp, indent=2) + "\n").encode()
    marker = (b"# agentboardGenerated sha256=" + hashlib.sha256(config_bytes).hexdigest().encode()
              + b" mcp-sha256=" + hashlib.sha256(mcp_bytes).hexdigest().encode() + b"\n")
    return marker + config_bytes, mcp_bytes, selected_provider


def toml_bytes(data):
    """Serialize the TOML value subset used by client configs; reject unknown values."""
    def key(name):
        return name if re.fullmatch(r"[A-Za-z0-9_-]+", name) else json.dumps(name)

    def value(item):
        if isinstance(item, str): return json.dumps(item, ensure_ascii=False)
        if isinstance(item, bool): return "true" if item else "false"
        if isinstance(item, (int, float)): return repr(item)
        if isinstance(item, list): return "[" + ", ".join(value(x) for x in item) + "]"
        raise ValueError(f"unsupported value in ~/.codewhale/config.toml: {type(item).__name__}")

    lines = []
    def table(obj, prefix=()):
        scalars = [(k, v) for k, v in obj.items() if not isinstance(v, dict) and not (isinstance(v, list) and v and isinstance(v[0], dict))]
        if prefix:
            lines.append("[" + ".".join(key(k) for k in prefix) + "]")
        for k, v in scalars: lines.append(f"{key(k)} = {value(v)}")
        for k, v in obj.items():
            if isinstance(v, dict):
                if lines and lines[-1] != "": lines.append("")
                table(v, prefix + (k,))
            elif isinstance(v, list) and v and isinstance(v[0], dict):
                for entry in v:
                    if lines and lines[-1] != "": lines.append("")
                    lines.append("[[" + ".".join(key(x) for x in prefix + (k,)) + "]]")
                    for ek, ev in entry.items():
                        if isinstance(ev, dict) or (isinstance(ev, list) and ev and isinstance(ev[0], dict)):
                            raise ValueError("unsupported nested table in ~/.codewhale/config.toml")
                        lines.append(f"{key(ek)} = {value(ev)}")
    table(data)
    return ("\n".join(lines) + "\n").encode()


def codewhale_file_state(path, want, owned, force, trusted_refresh=False):
    """new / unchanged / rewrite / differs, or None when the file is not ours."""
    if not os.path.lexists(path):
        return "new"
    if os.path.islink(path):
        if force:
            return "rewrite"
        print(f"not generated by agentboard: {path}; use --force to replace it", file=sys.stderr)
        return None
    if not os.path.isfile(path):
        print(f"not generated by agentboard: {path}; move it aside", file=sys.stderr)
        return None
    if not owned and not force:
        print(f"not generated by agentboard: {path}; use --force to replace it", file=sys.stderr)
        return None
    current = read(path)
    if current == want:
        return "unchanged" if os.stat(path).st_mode & 0o777 == 0o600 else "rewrite"
    marker = re.match(rb"# agentboardGenerated sha256=([0-9a-f]{64})(?: mcp-sha256=[0-9a-f]{64})?\n", current)
    if marker and hashlib.sha256(current[marker.end():]).hexdigest().encode() == marker.group(1):
        return "rewrite"
    if trusted_refresh:
        return "rewrite"
    if force:
        return "rewrite"
    print(f"unchanged: {path} (differs; --force rewrites it)")
    print(f"note: {path} differs from the template; --force rewrites it", file=sys.stderr)
    return "differs"


def codewhale_auth_key(provider, home):
    env = os.environ.copy()
    env["HOME"] = home
    env.pop("CODEWHALE_HOME", None)
    try:
        result = subprocess.run(
            ["codewhale", "auth", "print-api-key", "--provider", provider],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            check=False,
        )
    except OSError as exc:
        raise ValueError(f"cannot retrieve API key for provider {provider} with Codewhale auth CLI") from exc
    key = result.stdout.rstrip(b"\r\n")
    if result.returncode != 0 or not key or b"\n" in key or b"\r" in key or b"\0" in key:
        raise ValueError(f"cannot retrieve API key for provider {provider} with Codewhale auth CLI")
    return key


def codewhale_auth_inventory(home):
    """Return every runtime API-key route from Codewhale's value-free table."""
    env = os.environ.copy()
    env["HOME"] = home
    env.pop("CODEWHALE_HOME", None)
    try:
        result = subprocess.run(
            ["codewhale", "auth", "list"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            check=False,
        )
    except OSError as exc:
        raise ValueError("cannot enumerate Codewhale provider auth with auth list") from exc
    if result.returncode != 0:
        raise ValueError("cannot enumerate Codewhale provider auth with auth list")
    try:
        lines = result.stdout.decode("utf-8").splitlines()
    except UnicodeDecodeError as exc:
        raise ValueError("cannot parse Codewhale provider auth inventory") from exc
    header = re.compile(r"^provider\s+config\s+store\s+env\s+route$")
    if not lines or not header.fullmatch(lines[0].strip()):
        raise ValueError("cannot parse Codewhale provider auth inventory")
    routes = []
    seen = set()
    row_pattern = re.compile(
        r"^([A-Za-z0-9_.-]+)\s+(yes|no)\s+(yes|no)\s+(yes|no)\s+"
        r"(config|store|env|oauth|external|external-consent|missing)(?:\s+\([^()]*\))?$"
    )
    for line in lines[1:]:
        if not line.strip():
            continue
        match = row_pattern.fullmatch(line.strip())
        if not match:
            raise ValueError("cannot parse Codewhale provider auth inventory")
        provider, config, store, environment, route = match.groups()
        if provider in seen:
            raise ValueError("cannot parse Codewhale provider auth inventory")
        seen.add(provider)
        sources = {"config": config, "store": store, "env": environment}
        has_source = "yes" in sources.values()
        if route in sources:
            if sources[route] != "yes":
                raise ValueError("uncertain Codewhale provider auth inventory")
            routes.append(provider)
        elif route == "missing" and has_source:
            raise ValueError("uncertain Codewhale provider auth inventory")
    return routes


def write_private_file(path, data):
    os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "wb") as f:
        f.write(data)


def copy_codewhale_home(source, destination, replace_links=()):
    if not os.path.lexists(source):
        os.makedirs(destination, mode=0o700)
        return
    if os.path.islink(source) or not os.path.isdir(source):
        raise ValueError(f"invalid Codewhale seat home: {source}")
    os.makedirs(destination, mode=0o700)
    for root, dirs, files in os.walk(source, followlinks=False):
        relative = os.path.relpath(root, source)
        target_root = destination if relative == "." else os.path.join(destination, relative)
        os.makedirs(target_root, mode=0o700, exist_ok=True)
        os.chmod(target_root, 0o700)
        for name in dirs:
            src_dir = os.path.join(root, name)
            if os.path.islink(src_dir):
                raise ValueError(f"unsupported entry in Codewhale seat home: {src_dir}")
            os.makedirs(os.path.join(target_root, name), mode=0o700, exist_ok=True)
        for name in files:
            src_file = os.path.join(root, name)
            if os.path.islink(src_file) and os.path.relpath(src_file, source) in replace_links:
                continue
            if os.path.islink(src_file) or not os.path.isfile(src_file):
                raise ValueError(f"unsupported entry in Codewhale seat home: {src_file}")
            write_private_file(os.path.join(target_root, name), read(src_file))


def stage_codewhale_auth(provider_keys, home, codewhale_files, force, legacy_homes):
    scratch = "/tmp/agentboard/scratch"
    os.makedirs(scratch, mode=0o700, exist_ok=True)
    by_home = {}
    for path, want, state in codewhale_files:
        by_home.setdefault(os.path.dirname(path), {})[os.path.basename(path)] = (path, want, state)
    staged_artifacts = []
    updated_files = []
    env = os.environ.copy()
    env["HOME"] = home
    try:
        with tempfile.TemporaryDirectory(prefix="T404-codewhale-auth.", dir=scratch) as stage_root:
            os.chmod(stage_root, 0o700)
            for seat_home, outputs in by_home.items():
                seat = os.path.basename(os.path.dirname(seat_home))
                stage_home = os.path.join(stage_root, seat)
                replace_links = ("config.toml", "mcp.json") if force else ()
                source_home = legacy_homes.get(seat) or seat_home
                copy_codewhale_home(source_home, stage_home, replace_links)
                for filename, (target, want, state) in outputs.items():
                    write_private_file(os.path.join(stage_home, filename), want)
                stage_config = os.path.join(stage_home, "config.toml")
                stage_mcp = os.path.join(stage_home, "mcp.json")
                env["CODEWHALE_HOME"] = stage_home
                try:
                    for provider, key in provider_keys:
                        result = subprocess.run(
                            ["codewhale", "auth", "set", "--provider", provider, "--api-key-stdin"],
                            input=key,
                            stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE,
                            env=env,
                            check=False,
                        )
                        if result.returncode != 0:
                            raise ValueError(f"cannot stage API key for provider {provider} in {seat} home")
                except OSError as exc:
                    raise ValueError("cannot stage Codewhale provider API keys in seat home") from exc
                config_data = read(stage_config)
                if any(key in config_data for _, key in provider_keys):
                    raise ValueError("Codewhale auth CLI placed an API key in generated config")
                mcp_data = read(stage_mcp)
                mcp_want = outputs["mcp.json"][1]
                if mcp_data != mcp_want:
                    raise ValueError(f"Codewhale auth CLI changed generated MCP config for {seat}")
                marker = re.match(rb"# agentboardGenerated sha256=[0-9a-f]{64} mcp-sha256=[0-9a-f]{64}\n", config_data)
                body = config_data[marker.end():] if marker else config_data
                try:
                    parsed = tomllib.loads(body.decode("utf-8"))
                    normalized = toml_bytes(parsed)
                except (UnicodeDecodeError, tomllib.TOMLDecodeError, ValueError) as exc:
                    raise ValueError(f"Codewhale auth CLI produced invalid config for {seat}") from exc
                new_config = (
                    b"# agentboardGenerated sha256=" + hashlib.sha256(normalized).hexdigest().encode()
                    + b" mcp-sha256=" + hashlib.sha256(mcp_want).hexdigest().encode() + b"\n"
                    + normalized
                )
                for target, want, state in outputs.values():
                    final_want = new_config if target.endswith("/config.toml") else want
                    if state == "unchanged" and read(target) != final_want:
                        state = "rewrite"
                    updated_files.append((target, final_want, state))
                for root, dirs, files in os.walk(stage_home, followlinks=False):
                    for dirname in dirs:
                        entry = os.path.join(root, dirname)
                        if os.path.islink(entry):
                            raise ValueError("unsupported Codewhale auth artifact")
                        os.chmod(entry, 0o700)
                    for filename in files:
                        source_path = os.path.join(root, filename)
                        if os.path.islink(source_path) or not os.path.isfile(source_path):
                            raise ValueError("unsupported Codewhale auth artifact")
                        os.chmod(source_path, 0o600)
                        relative = os.path.relpath(source_path, stage_home)
                        if relative in ("config.toml", "mcp.json"):
                            continue
                        target_path = os.path.join(seat_home, relative)
                        payload = read(source_path)
                        if (
                            os.path.isfile(target_path)
                            and read(target_path) == payload
                            and os.stat(target_path).st_mode & 0o777 == 0o600
                        ):
                            continue
                        staged_artifacts.append((seat_home, target_path, payload))
    finally:
        env.pop("CODEWHALE_HOME", None)
    return updated_files, staged_artifacts


def install_private_auth_artifact(seat_home, path, payload):
    os.makedirs(seat_home, mode=0o700, exist_ok=True)
    os.chmod(seat_home, 0o700)
    relative_parent = os.path.relpath(os.path.dirname(path), seat_home)
    current = seat_home
    if relative_parent != ".":
        for part in relative_parent.split(os.sep):
            current = os.path.join(current, part)
            os.makedirs(current, mode=0o700, exist_ok=True)
            os.chmod(current, 0o700)
    tmp = f"{path}.agentboard.{os.getpid()}"
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "wb") as f:
            f.write(payload)
        os.replace(tmp, path)
    finally:
        if os.path.lexists(tmp):
            os.unlink(tmp)


def agent_list(value):
    names = value.split(",")
    if any(name not in AGENTS for name in names) or len(set(names)) != len(names):
        raise argparse.ArgumentTypeError(f"expects a comma-list of {','.join(AGENTS)} without duplicates")
    return names


def report_shared_leftovers(home, src):
    """Name files an older installer left in default client homes; never touch them."""
    def repo_link(path):
        return os.path.islink(path) and os.readlink(path).startswith(src + "/")

    found = []
    for seat in SEATS:
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
}


def run_cli(argv):
    return subprocess.run(argv, stdin=subprocess.DEVNULL, capture_output=True, text=True)


def register_global_mcp(agent):
    """Replace the one global agentboard MCP entry through the client's own command."""
    if not shutil.which(agent):
        print(f"note: {agent} not found on PATH; agentboard MCP entry not registered for it", file=sys.stderr)
        return
    if agent == "claude":
        # `claude mcp add-json` refuses an existing name, so remove first.
        gone = run_cli(["claude", "mcp", "remove", "-s", "user", "agentboard"])
        if gone.returncode != 0 and "No MCP server named" not in gone.stdout + gone.stderr:
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
        "~/.agents/skills and generate isolated per-role "
        "seat homes under ~/.local/share/agentboard/seats. "
        "Each client's own `mcp add` registers one global agentboard MCP entry in its default home; "
        "nothing else is written there.",
    )
    parser.add_argument(
        "--agent",
        type=agent_list,
        action="append",
        metavar="AGENT[,AGENT...]",
        help=f"install only these clients: {','.join(AGENTS)} (default: all, plus the launcher link and systemd unit)",
    )
    parser.add_argument("--force", action="store_true", help="replace existing generated seat files and unmarked config")
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
    agents = set(selected_agents) if selective else set(AGENTS)
    install_codex = not selective or "codex" in agents
    install_claude = not selective or "claude" in agents
    install_codewhale = not selective or "codewhale" in agents
    install_grok = not selective or "grok" in agents

    grok_seat_plans = []
    codex_seat_plans = []
    codex_auth_missing = False
    codex_stale_auth = []

    refuse = False
    claude_plans = []
    codewhale_files = []
    codewhale_provider = None
    codewhale_auth_files = []

    try:
        skills = skill_plans(home, repo, force)
    except (OSError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        skills = []
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

    if install_codex:
        try:
            codex_seat_plans, codex_auth_missing, codex_stale_auth = codex_seat_home_plans(home, src, force)
        except (OSError, ValueError) as exc:
            print(str(exc), file=sys.stderr)
            refuse = True
    if install_claude:
        try:
            claude_plans = claude_home_plans(home, src, force)
        except (OSError, ValueError) as exc:
            print(str(exc), file=sys.stderr)
            refuse = True

    if install_codewhale:
        legacy_homes = {}
        for seat in SEATS:
            cw_home = f"{home}/.local/share/agentboard/seats/{seat}/codewhale"
            legacy_home = f"{home}/.local/share/agentboard/codewhale/{seat}"
            if not os.path.lexists(cw_home) and os.path.lexists(legacy_home):
                legacy_homes[seat] = legacy_home
            if os.path.lexists(cw_home) and (os.path.islink(cw_home) or not os.path.isdir(cw_home)):
                print(f"invalid Codewhale seat home: {cw_home}", file=sys.stderr)
                refuse = True
                continue
            try:
                config_want, mcp_want, selected_provider = codewhale_home_files(src, cw_home, seat)
            except (ValueError, OSError) as exc:
                print(str(exc), file=sys.stderr)
                refuse = True
                continue
            if codewhale_provider is None:
                codewhale_provider = selected_provider
            elif selected_provider != codewhale_provider:
                print("Codewhale seats resolved different providers", file=sys.stderr)
                refuse = True
            config_path, mcp_path = f"{cw_home}/config.toml", f"{cw_home}/mcp.json"
            config_owned = (
                os.path.isfile(config_path)
                and not os.path.islink(config_path)
                and read(config_path).startswith(CODEWHALE_MARK)
            )
            trusted_mcp_refresh = False
            if config_owned and os.path.isfile(mcp_path) and not os.path.islink(mcp_path):
                current_config = read(config_path)
                header = re.match(rb"# agentboardGenerated sha256=[0-9a-f]{64} mcp-sha256=([0-9a-f]{64})\n", current_config)
                trusted_mcp_refresh = bool(header and hashlib.sha256(current_config[header.end():]).hexdigest().encode() == re.search(rb"sha256=([0-9a-f]{64})", current_config).group(1)
                                            and hashlib.sha256(read(mcp_path)).hexdigest().encode() == header.group(1))
            for path, want, owned in ((config_path, config_want, config_owned), (mcp_path, mcp_want, trusted_mcp_refresh)):
                state = codewhale_file_state(path, want, owned, force, trusted_mcp_refresh if path == mcp_path else False)
                if state is None:
                    refuse = True
                codewhale_files.append((path, want, state))

    if install_grok:
        try:
            grok_seat_plans = grok_seat_home_plans(home, src, force)
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            print(str(exc), file=sys.stderr)
            refuse = True
    if not selective:
        check(f"{home}/.local/bin/agentboard-manager")
        check(f"{home}/.config/systemd/user/agentboard.service")
    if not refuse and install_codewhale:
        try:
            exportable = codewhale_auth_inventory(home)
            provider_keys = [(name, codewhale_auth_key(name, home)) for name in exportable]
            codewhale_files, codewhale_auth_files = stage_codewhale_auth(
                provider_keys, home, codewhale_files, force, legacy_homes
            )
        except (OSError, ValueError) as exc:
            print(str(exc), file=sys.stderr)
            refuse = True
    if codex_auth_missing:
        if force and not refuse:
            cleanup_codex_seat_homes(codex_seat_plans, codex_stale_auth)
        print(f"missing Codex base auth: {home}/.codex/auth.json; authenticate Codex before installing seats", file=sys.stderr)
        refuse = True
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
    if install_codex:
        install_codex_seat_homes(codex_seat_plans)
    if install_claude:
        install_claude_seat_homes(claude_plans, home)
    for path, want, state in codewhale_files:
        if state in ("new", "rewrite"):
            os.makedirs(os.path.dirname(path), mode=0o700, exist_ok=True)
            os.chmod(os.path.dirname(path), 0o700)
            tmp = f"{path}.agentboard.{os.getpid()}"
            try:
                fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                os.fchmod(fd, 0o600)
                with os.fdopen(fd, "wb") as f:
                    f.write(want)
                os.replace(tmp, path)
            finally:
                if os.path.lexists(tmp): os.unlink(tmp)
            print(f"{'replaced' if state == 'rewrite' else 'generated'}: {path}")
        elif state == "unchanged":
            print(f"unchanged: {path}")
    for seat_home, path, payload in codewhale_auth_files:
        install_private_auth_artifact(seat_home, path, payload)
    if not selective:
        link(f"{src}/agentboard-manager", f"{home}/.local/bin/agentboard-manager")
        link(f"{src}/systemd/agentboard.service", f"{home}/.config/systemd/user/agentboard.service")
    if install_grok:
        install_grok_seat_homes(grok_seat_plans)

    for agent, selected in (("codex", install_codex), ("claude", install_claude),
                            ("grok", install_grok), ("codewhale", install_codewhale)):
        if selected:
            register_global_mcp(agent)
    report_shared_leftovers(home, src)
    print("done: seat profiles installed; see seats/README.md")


main()
