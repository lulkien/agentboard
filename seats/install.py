#!/usr/bin/env python3
"""Link the seat cards into their client homes: one physical copy, in this repo.
The Codex rules are not a link: they are a marked block inside the user's own
rules/default.rules, which Codex also appends to.
usage: install.py [--force]
"""
import os
import shutil
import sys

SEATS = ["analyst", "coder", "manager", "reviewer"]
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


def rules_scan(data):
    """("none",), ("ok", begin_line, end_line) or ("bad", "3,5").
    A marker is a whole line, ignoring one trailing CR; a line that matches only
    after trimming space and tab counts as a broken marker, never as a marker."""
    nb = ne = bad = fb = fe = 0
    found = []
    for nr, raw in enumerate(split_lines(data), 1):
        line = strip_cr(raw)
        t = line.strip(b" \t")
        if line == BEGIN:
            nb += 1
            fb = fb or nr
            found.append(nr)
        elif line == END:
            ne += 1
            fe = fe or nr
            found.append(nr)
        elif t == BEGIN or t == END:
            bad += 1
            found.append(nr)
    if nb + ne + bad == 0:
        return ("none",)
    if bad or nb != 1 or ne != 1 or fe < fb:
        return ("bad", ",".join(map(str, found)))
    return ("ok", fb, fe)


def source_ok(data):
    lines = [strip_cr(x) for x in split_lines(data)]
    markers = sum(1 for x in lines if x.strip(b" \t") in (BEGIN, END))
    return markers == 2 and lines[0] == BEGIN and lines[-1] == END


def read(path):
    with open(path, "rb") as f:
        return f.read()


def main():
    force = False
    for arg in sys.argv[1:]:
        if arg == "--force":
            force = True
        else:
            print("usage: install.py [--force]", file=sys.stderr)
            sys.exit(2)
    src = os.path.dirname(os.path.abspath(__file__))
    home = os.environ["HOME"]
    codex_home = os.environ.get("CODEX_HOME") or f"{home}/.codex"
    claude_home = f"{home}/.claude"
    opencode_home = f"{home}/.config/opencode"
    repo = os.path.dirname(src)

    rules_src = f"{src}/codex/rules/agentboard.rules"
    rules_dir = f"{codex_home}/rules"
    rules_file = f"{rules_dir}/default.rules"
    rules_tmp = f"{rules_dir}/.default.rules.agentboard.{os.getpid()}"

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

    for seat in SEATS:
        check(f"{codex_home}/{seat}.config.toml")

    # default.rules is judged here, with the other checks, so a refusal links nothing.
    if not source_ok(read(rules_src)):
        print(
            f"bad rules source: {rules_src} (must start with BEGIN, end with END, "
            "and hold no other marker)",
            file=sys.stderr,
        )
        refuse = True
    elif os.path.islink(rules_file):
        print(f"{rules_file} is a symlink; the installer edits only a regular file", file=sys.stderr)
        refuse = True
    elif os.path.exists(rules_file) and not os.path.isfile(rules_file):
        print(
            f"{rules_file} is not a regular file; the installer edits only a regular file",
            file=sys.stderr,
        )
        refuse = True
    elif os.path.isfile(rules_file):
        scan = rules_scan(read(rules_file))
        if scan[0] == "bad":
            print(
                f"malformed agentboard block in {rules_file} (marker lines {scan[1]}); "
                "fix it by hand",
                file=sys.stderr,
            )
            refuse = True
    for seat in SEATS:
        check(f"{claude_home}/{seat}-mcp.json")
    check(f"{opencode_home}/agentboard-mcp.sh")
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

    # Put the repo's block into default.rules: append when absent, replace in place
    # only under --force, never touch anything outside the marker lines.
    def install_rules_block():
        os.makedirs(rules_dir, exist_ok=True)
        exists = os.path.isfile(rules_file)
        before = read(rules_file) if exists else b""
        state = rules_scan(before) if exists else ("missing",)
        first = before.find(b"\n")
        nl = b"\r\n" if first > 0 and before[first - 1 : first] == b"\r" else b"\n"
        src_data = read(rules_src)
        block = b"".join(strip_cr(x) + nl for x in split_lines(src_data))
        ok = state[0] == "ok"
        if ok and not force:
            print(f"unchanged: {rules_file} (agentboard block present; --force rewrites it)")
            lines = split_lines(before)
            existing = b"\n".join(lines[state[1] - 1 : state[2]]).replace(b"\r", b"")
            if existing.rstrip(b"\n") != src_data.replace(b"\r", b"").rstrip(b"\n"):
                print(
                    f"note: agentboard block in {rules_file} differs from the repo; use --force",
                    file=sys.stderr,
                )
            return
        verb = "replaced" if ok else "added"
        if not exists:
            new = block
        elif ok:
            raw = raw_lines(before)
            new = b"".join(raw[: state[1] - 1]) + block + b"".join(raw[state[2] :])
        else:
            sep = nl if before and not before.endswith(b"\n") else b""
            new = before + sep + block
        if exists:
            shutil.copy2(rules_file, rules_tmp)  # cp -p: keep the file's mode
        with open(rules_tmp, "wb") as f:
            f.write(new)
        if exists and new == before:
            os.unlink(rules_tmp)
            print(f"unchanged: {rules_file} (agentboard block already current)")
            return
        if exists and read(rules_file) != before:
            print(
                f"{rules_file} changed during install; rerun with no Codex session open",
                file=sys.stderr,
            )
            sys.exit(1)
        os.replace(rules_tmp, rules_file)
        print(f"{verb}: {rules_file} (agentboard block)")

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
        link(f"{src}/opencode/agentboard-mcp.sh", f"{opencode_home}/agentboard-mcp.sh")
    finally:
        if os.path.lexists(rules_tmp):
            os.unlink(rules_tmp)

    base = f"{codex_home}/config.toml"
    if os.path.exists(base) and b"agentboard" not in read(base):
        print(f"note: merge {src}/codex/base.config.toml into {base}", file=sys.stderr)
    print("done: the codex base config is merged by hand; see seats/README.md")


main()
