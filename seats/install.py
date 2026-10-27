#!/usr/bin/env python3
"""Link the seat cards into their client homes: one physical copy, in this repo.
The Codex rules are not a link: they are a marked block inside the user's own
rules/default.rules, which Codex also appends to. Grok uses a marked block in its
user config as well.
usage: install.py [--force] [--grok-only]
"""
import os
import shutil
import sys
import tomllib

SEATS = ["analyst", "coder", "manager", "reviewer"]
BEGIN = b"# BEGIN AGENTBOARD RULES"
END = b"# END AGENTBOARD RULES"
GROK_BEGIN = b"# BEGIN AGENTBOARD MCP"
GROK_END = b"# END AGENTBOARD MCP"


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


def block_bytes(source, begin, end, nl):
    return b"".join(strip_cr(x) + nl for x in split_lines(source))


def marked_candidate(before, exists, state, source, begin, end):
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


def install_marked_block(path, source, begin, end, force, label, tmp=None, changed_context="client"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    exists = os.path.isfile(path)
    before = read(path) if exists else b""
    state = marked_scan(before, begin, end) if exists else ("missing",)
    ok = state[0] == "ok"
    if ok and not force:
        print(f"unchanged: {path} ({label} block present; --force rewrites it)")
        lines = split_lines(before)
        existing = b"\n".join(lines[state[1] - 1 : state[2]]).replace(b"\r", b"")
        if existing.rstrip(b"\n") != source.replace(b"\r", b"").rstrip(b"\n"):
            print(f"note: {label} block in {path} differs from the repo; use --force", file=sys.stderr)
        return
    verb = "replaced" if ok else "added"
    new = marked_candidate(before, exists, state, source, begin, end)
    tmp = tmp or f"{path}.agentboard.{os.getpid()}"
    if exists:
        shutil.copy2(path, tmp)
    with open(tmp, "wb") as f:
        f.write(new)
    if exists and new == before:
        os.unlink(tmp)
        print(f"unchanged: {path} ({label} block already current)")
        return
    if exists and read(path) != before:
        print(f"{path} changed during install; rerun with no {changed_context} open", file=sys.stderr)
        sys.exit(1)
    os.replace(tmp, path)
    print(f"{verb}: {path} ({label} block)")


def grok_preflight(path, source):
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
        print(
            f"unmarked [mcp_servers.agentboard] entry in {path}; review and remove or convert it by hand",
            file=sys.stderr,
        )
        return False, None
    return True, scan


def main():
    force = False
    grok_only = False
    i = 1
    while i < len(sys.argv):
        arg = sys.argv[i]
        if arg == "--force":
            force = True
        elif arg == "--grok-only":
            grok_only = True
        else:
            print("usage: install.py [--force] [--grok-only]", file=sys.stderr)
            sys.exit(2)
        i += 1
    src = os.path.dirname(os.path.abspath(__file__))
    home = os.environ["HOME"]
    codex_home = os.environ.get("CODEX_HOME") or f"{home}/.codex"
    claude_home = f"{home}/.claude"
    repo = os.path.dirname(src)

    rules_src = f"{src}/codex/rules/agentboard.rules"
    rules_dir = f"{codex_home}/rules"
    rules_file = f"{rules_dir}/default.rules"
    rules_tmp = f"{rules_dir}/.default.rules.agentboard.{os.getpid()}"
    grok_source_path = f"{src}/grok/agentboard-mcp.toml"
    grok_file = f"{home}/.grok/config.toml"

    refuse = False

    def check(p):
        nonlocal refuse
        if os.path.islink(p):
            target = os.readlink(p)
            if not target.startswith(repo + "/") and not force:
                print(f"foreign link: {p} -> {target} (use --force to replace it)", file=sys.stderr)
                refuse = True
        elif os.path.exists(p) and not force:
            print(f"exists: {p} (use --force)", file=sys.stderr)
            refuse = True

    if not grok_only:
        for seat in SEATS:
            check(f"{codex_home}/{seat}.config.toml")

    # default.rules is judged here, with the other checks, so a refusal links nothing.
    if not grok_only and not source_ok(read(rules_src)):
        print(
            f"bad rules source: {rules_src} (must start with BEGIN, end with END, "
            "and hold no other marker)",
            file=sys.stderr,
        )
        refuse = True
    elif not grok_only and os.path.islink(rules_file):
        print(f"{rules_file} is a symlink; the installer edits only a regular file", file=sys.stderr)
        refuse = True
    elif not grok_only and os.path.exists(rules_file) and not os.path.isfile(rules_file):
        print(
            f"{rules_file} is not a regular file; the installer edits only a regular file",
            file=sys.stderr,
        )
        refuse = True
    elif not grok_only and os.path.isfile(rules_file):
        scan = rules_scan(read(rules_file))
        if scan[0] == "bad":
            print(
                f"malformed agentboard block in {rules_file} (marker lines {scan[1]}); "
                "fix it by hand",
                file=sys.stderr,
            )
            refuse = True
    grok_source = read(grok_source_path)
    grok_ok, grok_state = grok_preflight(grok_file, grok_source)
    if not grok_ok:
        refuse = True
    grok_rendered = grok_source
    if grok_ok:
        current = read(grok_file) if os.path.isfile(grok_file) else b""
        if grok_state[0] == "ok" and not force:
            candidate = current
        else:
            candidate = marked_candidate(
                current,
                os.path.isfile(grok_file),
                grok_state,
                grok_rendered,
                GROK_BEGIN,
                GROK_END,
            )
        try:
            tomllib.loads(candidate.decode("utf-8"))
        except (UnicodeDecodeError, tomllib.TOMLDecodeError) as exc:
            print(f"invalid TOML in rendered Grok config {grok_file}: {exc}", file=sys.stderr)
            refuse = True
    if not grok_only:
        for seat in SEATS:
            check(f"{claude_home}/{seat}-mcp.json")
    if refuse:
        sys.exit(1)

    def link(s, d):
        os.makedirs(os.path.dirname(d), exist_ok=True)
        try:
            os.unlink(d)  # rm -f: a real directory must still fail
        except FileNotFoundError:
            pass
        os.symlink(s, d)
        print(f"linked: {d} -> {s}")

    def install_grok_block():
        install_marked_block(grok_file, grok_rendered, GROK_BEGIN, GROK_END, force, "agentboard MCP")

    if grok_only:
        install_grok_block()
        return

    # Put the repo's block into default.rules: append when absent, replace in place
    # only under --force, never touch anything outside the marker lines.
    def install_rules_block():
        install_marked_block(
            rules_file,
            read(rules_src),
            BEGIN,
            END,
            force,
            "agentboard",
            rules_tmp,
            "Codex session",
        )

    # The old install linked rules/agentboard.rules; Codex would load it as well.
    def remove_legacy_rules_link():
        legacy = f"{rules_dir}/agentboard.rules"
        if os.path.islink(legacy):
            if os.readlink(legacy).startswith(repo + "/"):
                os.unlink(legacy)
                print(f"removed: {legacy} (now a block in default.rules)")
            elif force:
                os.unlink(legacy)
                print(f"removed: {legacy} (foreign link)")
            else:
                print(f"note: foreign link {legacy}; --force removes it", file=sys.stderr)
        elif os.path.exists(legacy):
            print(
                f"note: {legacy} is a regular file; agentboard rules may load twice",
                file=sys.stderr,
            )

    try:
        for seat in SEATS:
            link(f"{src}/codex/{seat}.config.toml", f"{codex_home}/{seat}.config.toml")
        install_rules_block()
        remove_legacy_rules_link()
        for seat in SEATS:
            link(f"{src}/claude/{seat}-mcp.json", f"{claude_home}/{seat}-mcp.json")
        install_grok_block()
    finally:
        if os.path.lexists(rules_tmp):
            os.unlink(rules_tmp)

    base = f"{codex_home}/config.toml"
    if os.path.exists(base) and b"agentboard" not in read(base):
        print(f"note: merge {src}/codex/base.config.toml into {base}", file=sys.stderr)
    print("done: the codex base config is merged by hand; see seats/README.md")


main()
