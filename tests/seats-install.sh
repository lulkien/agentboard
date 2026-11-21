#!/usr/bin/env bash
# seats/install.py against isolated homes: foreign links, and the managed block
# it keeps inside the user's rules/default.rules. Nothing touches a real home.
set -u
cd "$(dirname "$0")/.."
repo=$(pwd)
src=$repo/seats/codex/rules/agentboard.rules
launcher=$repo/seats/start-agentboard-manager
unit=$repo/seats/systemd/agentboard.service
grok_src=$repo/seats/grok/agentboard-mcp.toml
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
n=0 fail=0
ok() { n=$((n + 1)); }
bad() { n=$((n + 1)); fail=$((fail + 1)); echo "FAIL: $*" >&2; }
is() { local name=$1; shift; if "$@"; then ok; else bad "$name"; fi; }
same() { local name=$1 a=$2 b=$3; if cmp -s "$a" "$b"; then ok; else bad "$name: $a differs from $b"; fi; }
has() { local name=$1 needle=$2 file=$3; if grep -qF -- "$needle" "$file"; then ok; else bad "$name: missing '$needle' in $(cat "$file")"; fi; }

cases=0
newhome() { # newhome <name>: a clean HOME with the dirs the installer expects
  cases=$((cases + 1))
  home=$tmp/h$cases
  rules=$home/.codex/rules/default.rules
  mkdir -p "$home/.claude" "$home/.codex"
  mkdir -p "$home/.agents/skills"
  for skill in agentboard-manager agentboard-analyst agentboard-coder agentboard-reviewer agentboard-team; do
    ln -s "$repo/skills/$skill" "$home/.agents/skills/$skill"
  done
  printf '{"tokens":{"access_token":"isolated-test-token"}}\n' >"$home/.codex/auth.json"
  chmod 600 "$home/.codex/auth.json"
}
inst() { # inst [args]: run the installer; stdout in $tmp/out, stderr in $tmp/err
  HOME="$home" CODEX_HOME="$home/.codex" PATH="$tmp/bin:$PATH" \
    AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" \
    AGENTBOARD_FAKE_AUTH_INVENTORY="$home/.codewhale/auth-inventory" \
    AGENTBOARD_FAKE_FAIL_LIST="${AGENTBOARD_FAKE_FAIL_LIST:-}" \
    AGENTBOARD_FAKE_FAIL_SEAT="${AGENTBOARD_FAKE_FAIL_SEAT:-}" \
    python3 ./seats/install.py "$@" >"$tmp/out" 2>"$tmp/err"
  local code=$?
  cat "$tmp/out" >>"$tmp/installer-stdout.log"
  cat "$tmp/err" >>"$tmp/installer-stderr.log"
  return "$code"
}
inst_with_codex_home() { # inst_with_codex_home <runtime home> [installer args]
  local runtime_home=$1; shift
  HOME="$home" CODEX_HOME="$runtime_home" PATH="$tmp/bin:$PATH" \
    AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" \
    AGENTBOARD_FAKE_AUTH_INVENTORY="$home/.codewhale/auth-inventory" \
    AGENTBOARD_FAKE_FAIL_LIST="${AGENTBOARD_FAKE_FAIL_LIST:-}" \
    AGENTBOARD_FAKE_FAIL_SEAT="${AGENTBOARD_FAKE_FAIL_SEAT:-}" \
    python3 ./seats/install.py "$@" >"$tmp/out" 2>"$tmp/err"
  local code=$?
  cat "$tmp/out" >>"$tmp/installer-stdout.log"
  cat "$tmp/err" >>"$tmp/installer-stderr.log"
  return "$code"
}
root_inst() { # root_inst [args]: exercise the supported repository entrypoint
  HOME="$home" CODEX_HOME="$home/.codex" PATH="$tmp/bin:$PATH" \
    AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" \
    AGENTBOARD_FAKE_AUTH_INVENTORY="$home/.codewhale/auth-inventory" \
    ./install.py "$@" >"$tmp/out" 2>"$tmp/err"
}
make_fake_codewhale() {
  mkdir -p "$tmp/bin"
  cat >"$tmp/bin/codewhale" <<'PY'
#!/usr/bin/env python3
import json
import os
import pathlib
import sys

args = sys.argv[1:]
with open(os.environ["AGENTBOARD_FAKE_AUTH_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(args) + "\n")
if args[:2] == ["auth", "list"]:
    if os.environ.get("AGENTBOARD_FAKE_FAIL_LIST"):
        sys.exit(7)
    inventory = pathlib.Path(os.environ["AGENTBOARD_FAKE_AUTH_INVENTORY"])
    if inventory.exists():
        sys.stdout.buffer.write(inventory.read_bytes())
    else:
        sys.stdout.write("provider config store env route\n")
    sys.exit(0)
if args[:2] == ["mcp", "list"]:
    config = pathlib.Path(os.environ["CODEWHALE_HOME"], "mcp.json")
    try:
        print(config.read_text(encoding="utf-8"))
    except OSError:
        sys.exit(3)
    sys.exit(0)
if args[:2] == ["config", "doctor"]:
    print("ok")
    sys.exit(0)
if args[:2] not in (["auth", "print-api-key"], ["auth", "set"]):
    sys.exit(2)
provider = args[args.index("--provider") + 1]
if args[:2] == ["auth", "print-api-key"]:
    destination = os.environ.get("CODEWHALE_HOME")
    stored = pathlib.Path(destination, "auth", provider + ".key") if destination else None
    source = pathlib.Path(os.environ["AGENTBOARD_FAKE_AUTH_DIR"], provider)
    path = stored if stored else source
    try:
        sys.stdout.buffer.write(path.read_bytes() + b"\n")
    except OSError:
        sys.exit(9)
elif args[:2] == ["auth", "set"]:
    seat = pathlib.Path(os.environ["CODEWHALE_HOME"]).name
    if os.environ.get("AGENTBOARD_FAKE_FAIL_SEAT") == seat:
        sys.exit(8)
    key = sys.stdin.buffer.read()
    path = pathlib.Path(os.environ["CODEWHALE_HOME"], "auth", provider + ".key")
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_bytes(key)
    path.chmod(0o600)
else:
    sys.exit(2)
PY
  chmod 700 "$tmp/bin/codewhale"
}
make_fake_codewhale
grok_inst() { # grok_inst [args]: Grok-only install into the isolated home
  HOME="$home" python3 ./seats/install.py --grok-only "$@" >"$tmp/out" 2>"$tmp/err"
}
grok_select() { # grok_select [args]: selected Grok install
  HOME="$home" CODEX_HOME="$home/.codex" python3 ./seats/install.py --agent grok "$@" >"$tmp/out" 2>"$tmp/err"
}
refused() { # refused <name> [args]: exit 1, nothing linked, default.rules byte-identical
  local name=$1; shift
  local before=$tmp/before
  if [ -e "$rules" ] && [ ! -L "$rules" ]; then cp "$rules" "$before"; else : >"$before"; fi
  inst "$@"; local code=$?
  is "$name exit" test "$code" -eq 1
  is "$name links nothing" test ! -e "$home/.codex/coder.config.toml"
  if [ -f "$rules" ] && [ ! -L "$rules" ]; then same "$name unchanged" "$before" "$rules"; fi
}

# --- isolated Codex role homes copy only validated base settings and auth -----
newhome
mkdir -p "$home/.codex/rules" "$home/.codex/sessions" "$home/.codex/cache"
cat >"$home/.codex/config.toml" <<'TOML'
model = "fixture-model"
approval_policy = "never"
sandbox_mode = "danger-full-access"

[mcp_servers.unrelated]
command = "fixture-mcp"

[hooks]
enabled = true

[plugins.fixture]
enabled = true

[projects."/fixture/project"]
trust_level = "trusted"
TOML
printf '{"tokens":{"access_token":"fixture-secret"}}\n' >"$home/.codex/auth.json"
printf 'host rule\n' >"$home/.codex/rules/default.rules"
printf 'runtime session\n' >"$home/.codex/sessions/session.jsonl"
printf 'runtime cache\n' >"$home/.codex/cache/state.sqlite"
cat >"$tmp/bin/codex" <<'PY'
#!/usr/bin/env python3
import os, sys, tomllib
args = sys.argv[1:]
assert len(args) == 2 and args[0] in ("-p", "--profile")
seat = args[1]
with open(os.path.join(os.environ["CODEX_HOME"], seat + ".config.toml"), "rb") as f:
    profile = tomllib.load(f)
print(profile["mcp_servers"]["agentboard"]["url"])
PY
chmod 700 "$tmp/bin/codex"
cp "$home/.codex/config.toml" "$tmp/base-config"
cp "$home/.codex/auth.json" "$tmp/base-auth"
config_sum=$(stat -c '%a:%s:%Y' "$home/.codex/config.toml")
auth_sum=$(stat -c '%a:%s:%Y' "$home/.codex/auth.json")
inst --agent codex --force; code=$?
is "isolated Codex install succeeds" test "$code" -eq 0
same "Codex base config unchanged" "$tmp/base-config" "$home/.codex/config.toml"
same "Codex base auth unchanged" "$tmp/base-auth" "$home/.codex/auth.json"
is "Codex base config mode and timestamp unchanged" test "$(stat -c '%a:%s:%Y' "$home/.codex/config.toml")" = "$config_sum"
is "Codex base auth mode and timestamp unchanged" test "$(stat -c '%a:%s:%Y' "$home/.codex/auth.json")" = "$auth_sum"
for seat in analyst coder manager reviewer; do
  codex_home="$home/.local/share/agentboard/seats/$seat/codex"
  same "$seat copied config preserves unrelated settings" "$tmp/base-config" "$codex_home/config.toml"
  same "$seat private auth snapshot content" "$tmp/base-auth" "$codex_home/auth.json"
  is "$seat home and auth are private" python3 -c 'import os,stat,sys; h,a=sys.argv[1:]; assert stat.S_IMODE(os.stat(h).st_mode)==0o700 and stat.S_IMODE(os.stat(a).st_mode)==0o600' "$codex_home" "$codex_home/auth.json"
  is "$seat profile selects correct MCP actor and role policy" python3 -c 'import sys,tomllib; s,p=sys.argv[1:]; d=tomllib.load(open(p,"rb")); assert d["mcp_servers"]["agentboard"]["url"].endswith("/mcp/"+s); assert d["approval_policy"] in ("never","on-request"); assert ("sandbox_mode" in d or "default_permissions" in d); assert d["mcp_servers"]["codebase-memory-mcp"]["default_tools_approval_mode"]=="approve"' "$seat" "$codex_home/$seat.config.toml"
  is "$seat Codex -p runtime smoke selects role home" bash -c 'test "$(HOME="$1" CODEX_HOME="$2" PATH="$3:$PATH" codex -p "$4")" = "http://127.0.0.1:8765/mcp/$4"' _ "$home" "$codex_home" "$tmp/bin" "$seat"
  is "$seat runtime files and host rules not copied" test ! -e "$codex_home/rules" -a ! -e "$codex_home/sessions" -a ! -e "$codex_home/cache"
done
is "reviewer profile remains restricted" python3 -c 'import tomllib,sys; d=tomllib.load(open(sys.argv[1],"rb")); assert d["default_permissions"]=="reviewer-scratch" and d["permissions"]["reviewer-scratch"]["extends"]==":read-only" and d["permissions"]["reviewer-scratch"]["filesystem"]["/tmp/agentboard/scratch"]=="write" and d["permissions"]["reviewer-scratch"]["network"]["enabled"] is True' "$home/.local/share/agentboard/seats/reviewer/codex/reviewer.config.toml"
is "analyst profile remains scratch-only" python3 -c 'import tomllib,sys; d=tomllib.load(open(sys.argv[1],"rb")); assert d["default_permissions"]=="seat-scratch" and d["permissions"]["seat-scratch"]["filesystem"]["/tmp/agentboard/scratch"]=="write"' "$home/.local/share/agentboard/seats/analyst/codex/analyst.config.toml"
is "private auth never printed" bash -c '! grep -q fixture-secret "$1" "$2"' _ "$tmp/out" "$tmp/err"

# A role CODEX_HOME is a runtime selector; installation cleans old shared
# profiles/rules from that restricted home and keeps compatibility files at ~/.codex.
reviewer_home="$home/.local/share/agentboard/seats/reviewer/codex"
ln -s "$repo/seats/codex/analyst.config.toml" "$reviewer_home/analyst.config.toml"
mv "$reviewer_home/reviewer.config.toml" "$tmp/reviewer-profile"
ln -s "$repo/seats/codex/reviewer.config.toml" "$reviewer_home/reviewer.config.toml"
mkdir -p "$reviewer_home/rules"
{
  printf 'host rule\n'
  cat "$repo/seats/codex/rules/agentboard.rules"
} >"$reviewer_home/rules/default.rules"
ln -s "$repo/seats/codex/rules/agentboard.rules" "$reviewer_home/rules/agentboard.rules"
inst_with_codex_home "$reviewer_home" --agent codex --force; code=$?
is "role CODEX_HOME install succeeds" test "$code" -eq 0
is "role profile is restored as a private regular file" test -f "$reviewer_home/reviewer.config.toml" -a ! -L "$reviewer_home/reviewer.config.toml"
is "legacy profile links removed from restricted home" test -z "$(find "$reviewer_home" -maxdepth 1 -type l -name '*.config.toml' -print -quit)"
is "legacy and inherited rules removed from restricted home" test ! -e "$reviewer_home/rules/default.rules" -a ! -e "$reviewer_home/rules/agentboard.rules"
is "restricted home rules directory removed when empty" test ! -e "$reviewer_home/rules"
is "compatibility links stay in base Codex home" test -L "$home/.codex/reviewer.config.toml" -a -f "$home/.codex/rules/default.rules"
same "reviewer role profile template restored" "$repo/seats/codex/reviewer.config.toml" "$reviewer_home/reviewer.config.toml"

# A CODEX_HOME symlink alias must be classified before installer writes. Role
# policy cleanup still targets the real home; compatibility writes stay at ~/.codex.
ln -s "$repo/seats/codex/analyst.config.toml" "$reviewer_home/analyst.config.toml"
rm "$reviewer_home/reviewer.config.toml"
ln -s "$repo/seats/codex/reviewer.config.toml" "$reviewer_home/reviewer.config.toml"
mkdir -p "$reviewer_home/rules"
{
  printf 'host rule\n'
  cat "$repo/seats/codex/rules/agentboard.rules"
} >"$reviewer_home/rules/default.rules"
ln -s "$repo/seats/codex/rules/agentboard.rules" "$reviewer_home/rules/agentboard.rules"
ln -s "$reviewer_home" "$tmp/codex-role-alias"
inst_with_codex_home "$tmp/codex-role-alias" --agent codex --force; code=$?
is "role-home CODEX_HOME alias install succeeds" test "$code" -eq 0
is "role-home alias leaves private profile regular" test -f "$reviewer_home/reviewer.config.toml" -a ! -L "$reviewer_home/reviewer.config.toml"
is "role-home alias removes legacy profile links" test -z "$(find "$reviewer_home" -maxdepth 1 -type l -name '*.config.toml' -print -quit)"
is "role-home alias removes inherited rules" test ! -e "$reviewer_home/rules/default.rules" -a ! -e "$reviewer_home/rules/agentboard.rules"
is "role-home alias keeps compatibility files in base home" test -L "$home/.codex/reviewer.config.toml" -a -f "$home/.codex/rules/default.rules"

# An alias into an unmanaged home cannot be safely classified. Refuse before
# touching files through it.
mkdir -p "$tmp/untrusted-codex-home"
ln -s "$tmp/untrusted-codex-home" "$tmp/codex-untrusted-alias"
inst_with_codex_home "$tmp/codex-untrusted-alias" --agent codex --force; code=$?
is "untrusted CODEX_HOME alias refuses" test "$code" -eq 1
has "untrusted CODEX_HOME alias diagnostic" "unsafe CODEX_HOME symlink alias" "$tmp/err"
is "untrusted CODEX_HOME alias writes no profiles or rules" test ! -e "$tmp/untrusted-codex-home/reviewer.config.toml" -a ! -e "$tmp/untrusted-codex-home/rules"

before_homes=$(find "$home/.local/share/agentboard/seats" -type f -print0 | sort -z | xargs -0 cksum | cksum)
inst --agent codex --force; code=$?
is "Codex force rerun succeeds" test "$code" -eq 0
after_homes=$(find "$home/.local/share/agentboard/seats" -type f -print0 | sort -z | xargs -0 cksum | cksum)
is "Codex force rerun is byte-idempotent" test "$before_homes" = "$after_homes"
is "selected Codex install leaves other clients untouched" test ! -e "$home/.claude/coder-mcp.json" -a ! -e "$home/.local/share/agentboard/seats/coder/codewhale/config.toml" -a ! -e "$home/.grok/config.toml"

# Missing base auth refuses new homes; force refresh removes revoked snapshots.
newhome
mkdir -p "$home/.codex/rules"
printf 'model = "fixture-model"\n' >"$home/.codex/config.toml"
rm "$home/.codex/auth.json"
inst --agent codex --force; code=$?
is "missing base auth refuses role-home generation" test "$code" -eq 1
has "missing base auth diagnostic" "missing Codex base auth: $home/.codex/auth.json" "$tmp/err"
is "missing base auth writes no role home" test ! -e "$home/.local/share/agentboard/seats"
is "missing base auth writes no legacy profile or rules" test ! -e "$home/.codex/coder.config.toml" -a ! -e "$home/.codex/rules/default.rules"

newhome
mkdir -p "$home/.codex/rules"
printf 'model = "fixture-model"\n' >"$home/.codex/config.toml"
printf '{"tokens":{"access_token":"revoked-fixture"}}\n' >"$home/.codex/auth.json"
inst --agent codex --force; code=$?
is "initial authenticated Codex role homes generate" test "$code" -eq 0
rm "$home/.codex/auth.json"
inst_with_codex_home "$home/.local/share/agentboard/seats/reviewer/codex" --agent codex --force; code=$?
is "force refresh without base auth refuses" test "$code" -eq 1
is "force refresh removes every stale credential snapshot" bash -c 'for s in analyst coder manager reviewer; do test ! -e "$1/.local/share/agentboard/seats/$s/codex/auth.json" || exit 1; done' _ "$home"
is "role CODEX_HOME still has no legacy profile links or rules" test ! -L "$home/.local/share/agentboard/seats/reviewer/codex/reviewer.config.toml" -a ! -e "$home/.local/share/agentboard/seats/reviewer/codex/rules"
is "base auth remains absent after revoked refresh" test ! -e "$home/.codex/auth.json"

# Malformed and unsafe base inputs refuse before any client destination changes.
for input in config auth; do
  newhome
  mkdir -p "$home/.codex/rules"
  if [[ $input == config ]]; then printf '[broken\n' >"$home/.codex/config.toml"; else printf '{broken' >"$home/.codex/auth.json"; fi
  inst --agent codex --force; code=$?
  is "malformed Codex $input refuses" test "$code" -eq 1
  is "malformed Codex $input writes no role home" test ! -e "$home/.local/share/agentboard/seats"
  is "malformed Codex $input writes no legacy profile" test ! -e "$home/.codex/coder.config.toml"
  is "malformed Codex $input writes no other client" test ! -e "$home/.claude/coder-mcp.json"
done
newhome
mkdir -p "$home/.codex/rules"
printf 'private target\n' >"$tmp/auth-target"
rm "$home/.codex/auth.json"
ln -s "$tmp/auth-target" "$home/.codex/auth.json"
inst --agent codex --force; code=$?
is "symlinked Codex auth refuses" test "$code" -eq 1
is "symlinked Codex auth target remains unchanged" test "$(cat "$tmp/auth-target")" = 'private target'
is "symlinked Codex auth writes no role home" test ! -e "$home/.local/share/agentboard/seats"

# --- generated Claude seat files: merge, idempotence and safe replacement ---
newhome
mkdir -p "$home/.codex/rules"
printf '%s' '{"account":{"fixture":"kept"},"projects":{"/private/project":{"hasTrustDialogAccepted":true,"trustLevel":"trusted","allowedTools":["Read"],"mcpServers":{"project-server":{"command":"private-project-mcp"}}}},"mcpServers":{"agentboard":{"type":"http","url":"http://wrong"},"extra":{"type":"stdio","command":"x","env":{"TOKEN":"s3cret-value"}}}}' >"$home/.claude.json"
cp "$home/.claude.json" "$tmp/claude-global-before"
cat >"$home/.claude/settings.json" <<'JSON'
{"env":{"CLAUDE_FIXTURE":"present"},"hooks":{"SessionStart":[]},"permissions":{"allow":["Read"],"deny":["Bash(rm *)"]},"enabledPlugins":{"fixture@market":"true"}}
JSON
cp "$home/.claude/settings.json" "$tmp/claude-settings-before"
mkdir -p "$home/.claude/plugins/cache/fixture/1"
printf '{"plugins":{"fixture@market":[{"installPath":"%s/.claude/plugins/cache/fixture/1"}]}}\n' "$home" >"$home/.claude/plugins/installed_plugins.json"
cp "$home/.claude/plugins/installed_plugins.json" "$tmp/claude-plugins-before"
printf 'fixture plugin implementation\n' >"$home/.claude/plugins/cache/fixture/1/plugin.js"
printf '{"accessToken":"synthetic-credential-sentinel"}\n' >"$home/.claude/.credentials.json"
chmod 600 "$home/.claude/.credentials.json"
cp "$home/.claude/.credentials.json" "$tmp/claude-credentials-before"
cp -r "$repo/seats/claude" "$tmp/cards-before"
inst; code=$?
is "generated install exit" test "$code" -eq 0
for seat in analyst coder manager reviewer; do
  is "$seat generated regular 0600 merged file" python3 -c 'import json,os,stat,sys; p,s=sys.argv[1:]; d=json.load(open(p)); card=json.load(open("seats/claude/"+s+"-mcp.json")); assert stat.S_IMODE(os.stat(p).st_mode)==0o600 and not os.path.islink(p); assert d["mcpServers"]["agentboard"]==card["mcpServers"]["agentboard"] and "extra" in d["mcpServers"]' "$home/.claude/$seat-mcp.json" "$seat"
  role="$home/.local/share/agentboard/seats/$seat/claude"
  is "$seat isolated private Claude home" test "$(stat -c %a "$role")" = 700
  is "$seat inherits settings and role restrictions" python3 -c 'import json,sys; d=json.load(open(sys.argv[1]+"/settings.json")); restricted=sys.argv[2] in ("analyst","reviewer"); assert d["env"]["CLAUDE_FIXTURE"]=="present" and "SessionStart" in d["hooks"] and d["permissions"]["deny"][0]=="Bash(rm *)" and d["permissions"]["allow"]==["Read"]; assert all((name in d["permissions"]["deny"])==restricted for name in ("Edit","Write","NotebookEdit")); assert d["enabledPlugins"]=={"fixture@market":"true"}' "$role" "$seat"
  is "$seat inherits global config without project trust" python3 -c 'import hashlib,json,os,stat,sys; p=sys.argv[1]; g=json.load(open(p+"/.claude.json")); c=open(p+"/.credentials.json","rb").read(); mark=open(p+"/.agentboard-credentials.sha256").read().strip(); assert g["account"]["fixture"]=="kept" and g["mcpServers"]["extra"]["env"]["TOKEN"]=="s3cret-value" and "projects" not in g; assert json.loads(c)["accessToken"]=="synthetic-credential-sentinel" and stat.S_IMODE(os.stat(p+"/.credentials.json").st_mode)==0o600 and mark==hashlib.sha256(c).hexdigest() and stat.S_IMODE(os.stat(p+"/.agentboard-credentials.sha256").st_mode)==0o600' "$role"
  is "$seat strict MCP card has role endpoint and user server" python3 -c 'import json,sys; d=json.load(open(sys.argv[1]+"/mcp.json"))["mcpServers"]; assert d["agentboard"]["url"].endswith("/mcp/"+sys.argv[2]) and d["extra"]["env"]["TOKEN"]=="s3cret-value"' "$role" "$seat"
  is "$seat canonical skills linked" test -L "$role/skills/agentboard-$seat" -a -L "$role/skills/agentboard-team" -a "$(readlink "$role/skills/agentboard-$seat")" = "$home/.agents/skills/agentboard-$seat"
  is "$seat does not copy plugin installation data" test ! -e "$role/plugins"
  is "$seat role files are private" bash -c 'for f in "$1"/settings.json "$1"/.claude.json "$1"/mcp.json "$1"/.credentials.json; do test "$(stat -c %a "$f")" = 600 || exit 1; done' _ "$role"
done
has "names only output" "servers: agentboard, codebase-memory-mcp, extra" "$tmp/out"
is "generated lines list names in sorted order" bash -c 'grep -q "servers: agentboard, codebase-memory-mcp, extra" "$1"' _ "$tmp/out"
is "secret not printed" bash -c '! grep -q s3cret-value "$1" "$2"' _ "$tmp/out" "$tmp/err"
same "user global config unchanged" "$tmp/claude-global-before" "$home/.claude.json"
same "user settings unchanged" "$tmp/claude-settings-before" "$home/.claude/settings.json"
same "plugin installation record unchanged" "$tmp/claude-plugins-before" "$home/.claude/plugins/installed_plugins.json"
same "user credentials unchanged" "$tmp/claude-credentials-before" "$home/.claude/.credentials.json"
diff -r "$tmp/cards-before" "$repo/seats/claude" >/dev/null || bad "repo cards unchanged"
# Existing role auth without a matching copied-snapshot hash is preserved even
# while base auth exists and --force refreshes the rest of the role home.
printf '{"accessToken":"base-present-role-login-sentinel"}\n' >"$home/.local/share/agentboard/seats/manager/claude/.credentials.json"
chmod 600 "$home/.local/share/agentboard/seats/manager/claude/.credentials.json"
cp "$home/.local/share/agentboard/seats/manager/claude/.credentials.json" "$tmp/manager-role-credentials-before"
inst --force; code=$?
is "force with base auth preserves unmarked role login" test "$code" -eq 0
same "base-present role login bytes preserved" "$tmp/manager-role-credentials-before" "$home/.local/share/agentboard/seats/manager/claude/.credentials.json"
has "base-present role login warning is actionable" "warning: preserving Claude role credentials with unknown ownership; inspect and remove manually if stale: $home/.local/share/agentboard/seats/manager/claude/.credentials.json" "$tmp/err"
is "base-present warning contains no credential or hash" bash -c '! grep -Eq "base-present-role-login-sentinel|[[:xdigit:]]{64}" "$1" "$2"' _ "$tmp/out" "$tmp/err"
inst
has "rerun unchanged" "unchanged: $home/.claude/coder-mcp.json" "$tmp/out"
has "no-force rerun warns about unmarked role login" "warning: preserving Claude role credentials with unknown ownership; inspect and remove manually if stale: $home/.local/share/agentboard/seats/manager/claude/.credentials.json" "$tmp/err"
same "no-force rerun preserves unmarked role login" "$tmp/manager-role-credentials-before" "$home/.local/share/agentboard/seats/manager/claude/.credentials.json"
cp "$home/.claude/coder-mcp.json" "$tmp/coder-before-refresh"
python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); d["mcpServers"]["extra"]["command"]="y"; open(p,"w").write(json.dumps(d))' "$home/.claude.json"
inst
has "stale generated file reported" "note: $home/.claude/coder-mcp.json differs" "$tmp/err"
same "stale generated file preserved without force" "$tmp/coder-before-refresh" "$home/.claude/coder-mcp.json"
inst --force
has "stale generated file refreshed with force" "replaced: $home/.claude/coder-mcp.json" "$tmp/out"
is "force copied updated server value" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["mcpServers"]["extra"]["command"]=="y"' "$home/.claude/coder-mcp.json"
rm "$home/.claude/reviewer-mcp.json"
inst
is "deleted generated file recreated" test -f "$home/.claude/reviewer-mcp.json"
is "all generated files regular" bash -c 'for f in "$1"/.claude/*-mcp.json; do test -f "$f" && test ! -L "$f" || exit 1; done' _ "$home"
is "all generated files private" bash -c 'for f in "$1"/.claude/*-mcp.json; do test "$(stat -c %a "$f")" = 600 || exit 1; done' _ "$home"
is "all generated markers present" bash -c 'for f in "$1"/.claude/*-mcp.json; do grep -q agentboardGenerated "$f" || exit 1; done' _ "$home"
is "card wins agentboard clash" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["mcpServers"]["agentboard"]["url"].endswith("/mcp/analyst")' "$home/.claude/analyst-mcp.json"
is "user extra server copied" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["mcpServers"]["extra"]["env"]["TOKEN"]=="s3cret-value"' "$home/.claude/analyst-mcp.json"
is "secret absent from output and errors" bash -c '! grep -q s3cret-value "$1" "$2"' _ "$tmp/out" "$tmp/err"
is "credential sentinel absent from output and errors" bash -c '! grep -q synthetic-credential-sentinel "$1" "$2"' _ "$tmp/out" "$tmp/err"
is "user config remains unchanged" grep -q 'wrong' "$home/.claude.json"
is "user config was not decorated with installer marker" bash -c '! grep -q agentboardGenerated "$1"' _ "$home/.claude.json"
inst
is "second run all unchanged" bash -c 'grep -c "^unchanged: .*/.claude/.*mcp.json$" "$1" | grep -qx 4' _ "$tmp/out"
is "unchanged file bytes stable" cmp -s "$home/.claude/coder-mcp.json" "$home/.claude/coder-mcp.json"
rm "$home/.claude/.credentials.json"
inst --agent claude; code=$?
is "stale role auth remains without force" test "$code" -eq 0 -a -f "$home/.local/share/agentboard/seats/analyst/claude/.credentials.json" -a -f "$home/.local/share/agentboard/seats/reviewer/claude/.credentials.json"
printf '{"accessToken":"independent-role-login-sentinel"}\n' >"$home/.local/share/agentboard/seats/coder/claude/.credentials.json"
chmod 600 "$home/.local/share/agentboard/seats/coder/claude/.credentials.json"
printf '{"accessToken":"legacy-unmarked-role-sentinel"}\n' >"$home/.local/share/agentboard/seats/reviewer/claude/.credentials.json"
chmod 600 "$home/.local/share/agentboard/seats/reviewer/claude/.credentials.json"
cp "$home/.local/share/agentboard/seats/reviewer/claude/.credentials.json" "$tmp/reviewer-role-credentials-before"
inst --agent claude --force; code=$?
is "force removes proven snapshot and preserves role logins" test "$code" -eq 0 -a ! -e "$home/.local/share/agentboard/seats/analyst/claude/.credentials.json" -a ! -e "$home/.local/share/agentboard/seats/analyst/claude/.agentboard-credentials.sha256" -a -f "$home/.local/share/agentboard/seats/coder/claude/.credentials.json" -a -f "$home/.local/share/agentboard/seats/manager/claude/.credentials.json" -a -f "$home/.local/share/agentboard/seats/reviewer/claude/.credentials.json"
is "independent role login bytes preserved" grep -q independent-role-login-sentinel "$home/.local/share/agentboard/seats/coder/claude/.credentials.json"
same "legacy unmarked role login bytes preserved" "$tmp/reviewer-role-credentials-before" "$home/.local/share/agentboard/seats/reviewer/claude/.credentials.json"
has "legacy unmarked role login warns for manual review" "warning: preserving Claude role credentials with unknown ownership; inspect and remove manually if stale: $home/.local/share/agentboard/seats/reviewer/claude/.credentials.json" "$tmp/err"
has "changed marked role login warns for manual review" "warning: preserving Claude role credentials with unknown ownership; inspect and remove manually if stale: $home/.local/share/agentboard/seats/coder/claude/.credentials.json" "$tmp/err"
has "changed role login warns after base auth withdrawal" "warning: preserving Claude role credentials with unknown ownership; inspect and remove manually if stale: $home/.local/share/agentboard/seats/manager/claude/.credentials.json" "$tmp/err"
is "withdrawal warnings contain no credential or hash" bash -c '! grep -Eq "synthetic-credential-sentinel|independent-role-login-sentinel|legacy-unmarked-role-sentinel|base-present-role-login-sentinel|[[:xdigit:]]{64}" "$1" "$2"' _ "$tmp/out" "$tmp/err"
is "force stale auth removal prints no credentials" bash -c '! grep -q synthetic-credential-sentinel "$1" "$2"' _ "$tmp/out" "$tmp/err"
is "repo cards remain regular" bash -c 'for f in seats/claude/*-mcp.json; do test -f "$f" && test ! -L "$f" || exit 1; done'
is "repo cards retain expected count" test "$(find seats/claude -maxdepth 1 -name '*-mcp.json' | wc -l)" -eq 4
is "Claude home file count four" test "$(find "$home/.claude" -maxdepth 1 -name '*-mcp.json' | wc -l)" -eq 4
is "user config lacks rewritten marker" bash -c '! grep -q agentboardGenerated "$1"' _ "$home/.claude.json"
is "file contains only server names in log" bash -c '! grep -q s3cret-value "$1"' _ "$tmp/out"
is "missing user config tolerated" test -f "$home/.claude/coder-mcp.json"
is "all seat endpoints distinct" python3 -c 'import glob,json,sys; u=[json.load(open(p))["mcpServers"]["agentboard"]["url"] for p in glob.glob(sys.argv[1]+"/.claude/*-mcp.json")]; assert len(u)==len(set(u))==4' "$home"

# Old repository symlinks are replaced at the directory entry, never followed.
newhome
mkdir -p "$home/.codex/rules"
ln -s "$repo/seats/claude/coder-mcp.json" "$home/.claude/coder-mcp.json"
inst; code=$?
is "repo card symlink install succeeds" test "$code" -eq 0
is "repo card symlink replaced by regular file" test -f "$home/.claude/coder-mcp.json" -a ! -L "$home/.claude/coder-mcp.json"
diff -r "$tmp/cards-before" "$repo/seats/claude" >/dev/null || bad "repo card symlink target untouched"
rm "$home/.claude/coder-mcp.json"
ln -s /old/checkout/coder-mcp.json "$home/.claude/coder-mcp.json"
inst; code=$?
is "foreign card symlink refused" test "$code" -eq 1
is "foreign link preserved on refusal" test "$(readlink "$home/.claude/coder-mcp.json")" = /old/checkout/coder-mcp.json
inst --force; code=$?
is "force replaces foreign card link" test "$code" -eq 0 -a -f "$home/.claude/coder-mcp.json" -a ! -L "$home/.claude/coder-mcp.json"
printf 'mine\n' >"$home/.claude/manager-mcp.json"
cp "$home/.claude/manager-mcp.json" "$tmp/claude-unmarked-before"
inst; code=$?
is "unmarked regular Claude file refuses without force" test "$code" -eq 1
same "unmarked Claude file preserved without force" "$tmp/claude-unmarked-before" "$home/.claude/manager-mcp.json"
inst --force; code=$?
is "force replaces unmarked regular Claude file" test "$code" -eq 0
is "forced Claude file is generated card" python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert "agentboardGenerated" in d and d["mcpServers"]["agentboard"]["url"].endswith("/mcp/manager")' "$home/.claude/manager-mcp.json"
rm "$home/.claude/manager-mcp.json"
printf '{' >"$home/.claude.json"
inst --force; code=$?
is "invalid user JSON refused" test "$code" -eq 1
has "invalid JSON diagnostic names file" "invalid JSON in $home/.claude.json" "$tmp/err"
printf '{"mcpServers":[]}' >"$home/.claude.json"
inst --force; code=$?
is "non-object mcpServers refused" test "$code" -eq 1
has "mcpServers diagnostic names field" 'mcpServers in' "$tmp/err"
# --- missing rules dir: the block alone, no second rules file ---------------
newhome
inst; code=$?
is "fresh install exit" test "$code" -eq 0
is "manager launcher linked" test "$(readlink "$home/.local/bin/start-agentboard-manager")" = "$launcher"
is "user unit linked" test "$(readlink "$home/.config/systemd/user/agentboard.service")" = "$unit"
for seat in analyst coder manager reviewer; do
  is "$seat Claude URL card" python3 -c 'import json,sys; s=sys.argv[1]; p=sys.argv[2]; a=json.load(open(p))["mcpServers"]["agentboard"]; assert a == {"type":"http","url":f"http://127.0.0.1:8765/mcp/{s}"}' "$seat" "$repo/seats/claude/$seat-mcp.json"
  is "$seat generated Claude file" test -f "$home/.claude/$seat-mcp.json"
  is "$seat Codex URL card" python3 -c 'import sys,tomllib; s=sys.argv[1]; p=sys.argv[2]; a=tomllib.load(open(p,"rb"))["mcp_servers"]["agentboard"]; assert a["url"] == f"http://127.0.0.1:8765/mcp/{s}" and "command" not in a and "env" not in a and len(a["tools"]) == 12' "$seat" "$home/.codex/$seat.config.toml"
  is "$seat Codex cbm approval only" python3 -c 'import sys,tomllib; a=tomllib.load(open(sys.argv[1],"rb"))["mcp_servers"]["codebase-memory-mcp"]; assert a == {"default_tools_approval_mode":"approve"}' "$home/.codex/$seat.config.toml"
  is "$seat Codex rtk pointer" python3 -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb"))["developer_instructions"]; assert "rtk <cmd>" in d and "command -v rtk" in d and "Rules for every agent" in d' "$home/.codex/$seat.config.toml"
done
has "unit restarts on failure" 'Restart=on-failure' "$home/.config/systemd/user/agentboard.service"
has "unit prevents exit one restart" 'RestartPreventExitStatus=1' "$home/.config/systemd/user/agentboard.service"
same "fresh default.rules is the source block" "$src" "$rules"
is "rules block rtk allows only git reads" python3 -c 'import re,sys; s=open(sys.argv[1]).read(); found={tuple(re.findall(r"\"([^\"]+)\"", m)) for m in re.findall(r"prefix_rule\(pattern=\[([^]]+)\]", s) if m.startswith("\"rtk\"")}; assert found == {("rtk","git","diff"),("rtk","git","log"),("rtk","git","show"),("rtk","git","status")}, found' "$src"
is "no separate agentboard.rules" test ! -e "$home/.codex/rules/agentboard.rules"
has "fresh install says added" "added: $rules" "$tmp/out"
has "done line kept" "done: seat profiles installed" "$tmp/out"

# --- manager launcher link follows the same foreign-link rules ---------------
newhome
mkdir -p "$home/.local/bin"
ln -s /old/checkout/start-agentboard-manager "$home/.local/bin/start-agentboard-manager"
inst; code=$?
is "foreign launcher link refused" test "$code" -eq 1
has "foreign launcher link diagnosed" 'foreign link:' "$tmp/err"
is "foreign launcher link preserved" test "$(readlink "$home/.local/bin/start-agentboard-manager")" = /old/checkout/start-agentboard-manager
is "foreign launcher refusal leaves unit unlinked" test ! -e "$home/.config/systemd/user/agentboard.service"
inst --force
is "force replaces launcher link" test "$(readlink "$home/.local/bin/start-agentboard-manager")" = "$launcher"

# --- user unit follows the same foreign-link rules ---------------------------
newhome
mkdir -p "$home/.config/systemd/user"
ln -s /old/checkout/agentboard.service "$home/.config/systemd/user/agentboard.service"
inst; code=$?
is "foreign unit link refused" test "$code" -eq 1
has "foreign unit link diagnosed" 'foreign link:' "$tmp/err"
is "foreign unit link preserved" test "$(readlink "$home/.config/systemd/user/agentboard.service")" = /old/checkout/agentboard.service
is "foreign unit refusal links no Codex card" test ! -e "$home/.codex/coder.config.toml"
inst --force
is "force replaces unit link" test "$(readlink "$home/.config/systemd/user/agentboard.service")" = "$unit"

# --force replaces regular Codex entries, but refuses directories before writing.
newhome
mkdir -p "$home/.codex/rules"
printf 'foreign profile\n' >"$home/.codex/coder.config.toml"
cp "$home/.codex/coder.config.toml" "$tmp/codex-foreign-before"
inst; code=$?
is "foreign regular Codex profile refuses without force" test "$code" -eq 1
same "foreign regular Codex profile preserved without force" "$tmp/codex-foreign-before" "$home/.codex/coder.config.toml"
inst --force; code=$?
is "force replaces regular Codex profile" test "$code" -eq 0 -a -L "$home/.codex/coder.config.toml"
is "Codex replacement points at this seat profile" test "$(readlink "$home/.codex/coder.config.toml")" = "$repo/seats/codex/coder.config.toml"

newhome
mkdir -p "$home/.codex/rules"
printf 'foreign target' >"$tmp/codex-target"
ln -s "$tmp/codex-target" "$home/.codex/coder.config.toml"
inst; code=$?
is "foreign Codex profile link refuses without force" test "$code" -eq 1
is "foreign Codex profile link remains without force" test -L "$home/.codex/coder.config.toml"
inst --force; code=$?
is "force replaces foreign Codex profile link" test "$code" -eq 0 -a -L "$home/.codex/coder.config.toml" -a "$(readlink "$home/.codex/coder.config.toml")" = "$repo/seats/codex/coder.config.toml"
is "foreign Codex profile link target stays unchanged" test "$(cat "$tmp/codex-target")" = 'foreign target'

newhome
mkdir -p "$home/.codex/rules" "$home/.claude"
mkdir "$home/.codex/coder.config.toml"
inst --force; code=$?
is "Codex directory refuses under force" test "$code" -eq 1
is "Codex directory survives force refusal" test -d "$home/.codex/coder.config.toml"
is "Codex directory refusal writes no Claude card" test ! -e "$home/.claude/coder-mcp.json"
is "Codex directory refusal writes no rules" test ! -e "$rules"

# --- grok-only does not manage the PATH launcher ------------------------------
newhome
grok_inst
is "grok-only leaves manager launcher alone" test ! -e "$home/.local/bin/start-agentboard-manager"
is "grok-only leaves user unit alone" test ! -e "$home/.config/systemd/user/agentboard.service"

# --- user rules, no trailing newline: original bytes stay the prefix ---------
newhome
mkdir -p "$home/.codex/rules"
printf 'prefix_rule(pattern=["ls"], decision="allow")' >"$rules"
cp "$rules" "$tmp/orig"
inst
is "append keeps the original bytes" cmp -s -n "$(wc -c <"$tmp/orig")" "$tmp/orig" "$rules"
is "append adds exactly one newline" test "$(head -c "$(($(wc -c <"$tmp/orig") + 1))" "$rules" | tail -c 1 | od -An -c | tr -d ' ')" = '\n'
tail -n "$(wc -l <"$src")" "$rules" >"$tmp/tail"
same "append ends with the block" "$src" "$tmp/tail"
sum=$(cksum <"$rules")
inst
is "second run is idempotent" test "$(cksum <"$rules")" = "$sum"
has "second run says unchanged" 'unchanged:' "$tmp/out"
inst --force
is "force on an identical block keeps bytes" test "$(cksum <"$rules")" = "$sum"
has "force on an identical block says unchanged" 'unchanged:' "$tmp/out"

# --- an edited block: plain run keeps it, --force replaces it in place -------
newhome
mkdir -p "$home/.codex/rules"
{
  echo 'prefix_rule(pattern=["before"], decision="allow")'
  sed 's/"agentboard"/"edited"/' "$src"
  echo 'prefix_rule(pattern=["after"], decision="allow")'
} >"$rules"
cp "$rules" "$tmp/edited"
inst
same "plain run leaves an edited block" "$tmp/edited" "$rules"
has "plain run notes the difference" 'differs from the repo' "$tmp/err"
has "plain run says unchanged" 'unchanged:' "$tmp/out"
inst --force
has "force says replaced" 'replaced:' "$tmp/out"
{
  echo 'prefix_rule(pattern=["before"], decision="allow")'
  cat "$src"
  echo 'prefix_rule(pattern=["after"], decision="allow")'
} >"$tmp/expect"
same "force replaces only the marked range, in place" "$tmp/expect" "$rules"

# --- malformed markers refuse, with and without --force ----------------------
B='# BEGIN AGENTBOARD RULES'
E='# END AGENTBOARD RULES'
n_case=0
while IFS='|' read -r label body; do
  for args in "" "--force"; do
    newhome
    mkdir -p "$home/.codex/rules"
    printf '%b' "$body" >"$rules"
    # shellcheck disable=SC2086
    refused "malformed $label ${args:-plain}" $args
    has "malformed $label ${args:-plain} text" 'malformed agentboard block' "$tmp/err"
  done
done <<EOF
duplicate begin|$B\nx\n$B\n$E\n
duplicate end|$B\n$E\n$E\n
begin only|a\n$B\nb\n
end only|a\n$E\nb\n
end before begin|$E\n$B\n
indented begin|  $B\nx\n$E\n
trailing space end|$B\nx\n$E \n
valid pair plus indented begin|$B\nx\n$E\n  $B\n
EOF

# --- CRLF files get a CRLF block, and a rerun changes nothing ----------------
newhome
mkdir -p "$home/.codex/rules"
printf 'prefix_rule(pattern=["ls"], decision="allow")\r\n' >"$rules"
inst
is "crlf block ends with a crlf end marker" test "$(tail -n 1 "$rules" | tr -d '\r')" = "$E" -a "$(tail -n 1 "$rules" | tail -c 2 | od -An -c | tr -d ' ')" = '\r\n'
is "crlf file has no bare lf" test "$(grep -c $'\r$' "$rules")" -eq "$(wc -l <"$rules")"
sum=$(cksum <"$rules")
inst
is "crlf rerun is idempotent" test "$(cksum <"$rules")" = "$sum"
inst --force
is "crlf force keeps a current block" test "$(cksum <"$rules")" = "$sum"

# --- a symlinked or non-file default.rules is never followed or replaced -----
newhome
mkdir -p "$home/.codex/rules"
printf 'target\n' >"$home/target.rules"
ln -s "$home/target.rules" "$rules"
for args in "" "--force"; do
  inst $args; code=$?
  is "symlink ${args:-plain} exit" test "$code" -eq 1
  is "symlink ${args:-plain} still a link" test "$(readlink "$rules")" = "$home/target.rules"
  is "symlink ${args:-plain} target unchanged" test "$(cat "$home/target.rules")" = target
  has "symlink ${args:-plain} text" 'is a symlink' "$tmp/err"
  is "symlink ${args:-plain} links nothing" test ! -e "$home/.codex/coder.config.toml"
done
newhome
mkdir -p "$rules"
inst --force; code=$?
is "directory default.rules exit" test "$code" -eq 1
has "directory default.rules text" 'not a regular file' "$tmp/err"

# --- mode is preserved on replace and append --------------------------------
newhome
mkdir -p "$home/.codex/rules"
cat "$src" >"$rules"
sed -i 's/"agentboard"/"edited"/' "$rules"
chmod 640 "$rules"
inst --force
is "replace keeps mode" test "$(stat -c %a "$rules")" = 640
newhome
mkdir -p "$home/.codex/rules"
printf 'x\n' >"$rules"
chmod 600 "$rules"
inst
is "append keeps mode" test "$(stat -c %a "$rules")" = 600
is "no temp file left" test -z "$(ls -A "$home/.codex/rules" | grep -F .default.rules.agentboard || true)"

# --- legacy rules/agentboard.rules ------------------------------------------
newhome
mkdir -p "$home/.codex/rules"
ln -s "$src" "$home/.codex/rules/agentboard.rules"
inst
is "repo-pointing legacy link removed" test ! -e "$home/.codex/rules/agentboard.rules" -a ! -L "$home/.codex/rules/agentboard.rules"
has "legacy removal reported" 'removed:' "$tmp/out"
newhome
mkdir -p "$home/.codex/rules"
ln -s /old/checkout/agentboard.rules "$home/.codex/rules/agentboard.rules"
inst
is "foreign legacy link kept" test -L "$home/.codex/rules/agentboard.rules"
has "foreign legacy link noted" 'foreign link' "$tmp/err"
inst --force
is "force removes a foreign legacy link" test ! -L "$home/.codex/rules/agentboard.rules"
newhome
mkdir -p "$home/.codex/rules"
printf 'mine\n' >"$home/.codex/rules/agentboard.rules"
inst --force
is "legacy regular file kept" test "$(cat "$home/.codex/rules/agentboard.rules")" = mine
has "legacy regular file noted" 'regular file' "$tmp/err"

# --- an empty rules source is refused, not a crash ---------------------------
newhome
mkdir -p "$tmp/fake/codex/rules"
cp -r seats/. "$tmp/fake/"
: >"$tmp/fake/codex/rules/agentboard.rules"
HOME="$home" CODEX_HOME="$home/.codex" python3 "$tmp/fake/install.py" >"$tmp/out" 2>"$tmp/err"; code=$?
is "empty rules source refused" test "$code" -eq 1
has "empty rules source named" 'bad rules source' "$tmp/err"
is "empty rules source links nothing" test ! -e "$home/.codex/coder.config.toml"

# --- isolated Grok role homes; legacy ~/.grok remains untouched --------------
newhome
mkdir -p "$home/.grok"
printf '[ui]\ntheme = "dark"\n' >"$home/.grok/config.toml"
printf '{"access_token":"synthetic"}\n' >"$home/.grok/auth.json"
printf '{"server":"initial-synthetic"}\n' >"$home/.grok/mcp_credentials.json"
chmod 600 "$home/.grok/auth.json"
cp "$home/.grok/config.toml" "$tmp/grok-before"
grok_inst; code=$?
is "fresh Grok role install exit" test "$code" -eq 0
same "legacy Grok config stays unchanged" "$tmp/grok-before" "$home/.grok/config.toml"
for seat in analyst coder manager reviewer; do
  role="$home/.local/share/agentboard/seats/$seat/grok"
  has "$seat Grok actor URL" "url = \"http://127.0.0.1:8765/mcp/$seat\"" "$role/config.toml"
  is "$seat Grok home private" test "$(stat -c %a "$role")" = 700
  is "$seat Grok auth private" test "$(stat -c %a "$role/auth.json")" = 600
  is "$seat Grok MCP credentials private" test "$(stat -c %a "$role/mcp_credentials.json")" = 600
done
cp "$home/.local/share/agentboard/seats/manager/grok/config.toml" "$tmp/grok-before"
grok_inst
same "Grok role config rerun is byte-identical" "$tmp/grok-before" "$home/.local/share/agentboard/seats/manager/grok/config.toml"

# A disabled edit inside the managed card must refuse plain install; force restores it.
newhome
grok_select; code=$?
is "fresh selected Grok role homes install" test "$code" -eq 0
role="$home/.local/share/agentboard/seats/reviewer/grok"
sed -i 's/enabled = true/enabled = false/' "$role/config.toml"
cp "$role/config.toml" "$tmp/disabled-grok-card"
grok_select; code=$?
is "plain selected Grok refuses disabled managed card" test "$code" -eq 1
same "plain refusal preserves edited card for inspection" "$tmp/disabled-grok-card" "$role/config.toml"
grok_select --force; code=$?
is "force restores managed Grok card" test "$code" -eq 0
is "force restores enabled agentboard server" python3 -c 'import sys,tomllib; c=tomllib.load(open(sys.argv[1],"rb")); assert c["mcp_servers"]["agentboard"]["enabled"] is True and c["mcp_servers"]["agentboard"]["url"].endswith("/mcp/reviewer")' "$role/config.toml"

# A legacy shared card's variable actor must become a literal role URL in each copy.
newhome
mkdir -p "$home/.grok"
cp "$grok_src" "$home/.grok/config.toml"
printf '\n[ui]\nnote = "keep /mcp/${AGENTBOARD_SEAT:-manager} unchanged"\n' >>"$home/.grok/config.toml"
grok_inst; code=$?
is "legacy Grok card role install succeeds" test "$code" -eq 0
for seat in analyst coder manager reviewer; do
  has "$seat copied legacy card has its fixed actor" "url = \"http://127.0.0.1:8765/mcp/$seat\"" "$home/.local/share/agentboard/seats/$seat/grok/config.toml"
  has "$seat copied legacy config preserves unrelated actor-looking text" 'note = "keep /mcp/${AGENTBOARD_SEAT:-manager} unchanged"' "$home/.local/share/agentboard/seats/$seat/grok/config.toml"
done

# Force updates only the managed MCP block and installer-owned credential snapshots.
role="$home/.local/share/agentboard/seats/reviewer/grok"
printf '\n[custom]\nurl = "https://user.invalid/${AGENTBOARD_SEAT:-manager}"\n' >>"$role/config.toml"
printf '{"independent":"reviewer-auth"}\n' >"$role/auth.json"
printf '{"independent":"reviewer-mcp"}\n' >"$role/mcp_credentials.json"
cp "$role/auth.json" "$tmp/diverged-auth"
cp "$role/mcp_credentials.json" "$tmp/diverged-mcp"
printf '{"access_token":"rotated-synthetic"}\n' >"$home/.grok/auth.json"
printf '{"server":"synthetic"}\n' >"$home/.grok/mcp_credentials.json"
grok_inst; code=$?
is "base credential change without force succeeds" test "$code" -eq 0
has "auth snapshot waits for force before refresh" 'synthetic' "$home/.local/share/agentboard/seats/analyst/grok/auth.json"
has "MCP snapshot waits for force before refresh" 'synthetic' "$home/.local/share/agentboard/seats/analyst/grok/mcp_credentials.json"
grok_inst --force; code=$?
is "force refreshes managed Grok home" test "$code" -eq 0
same "force preserves diverged role auth" "$tmp/diverged-auth" "$role/auth.json"
same "force preserves diverged role MCP credentials" "$tmp/diverged-mcp" "$role/mcp_credentials.json"
has "force preserves unrelated actor-looking role setting" 'url = "https://user.invalid/${AGENTBOARD_SEAT:-manager}"' "$role/config.toml"
has "force preserves role's fixed agentboard actor" 'url = "http://127.0.0.1:8765/mcp/reviewer"' "$role/config.toml"
for seat in analyst coder manager; do
  has "$seat force refreshes recorded auth snapshot" 'rotated-synthetic' "$home/.local/share/agentboard/seats/$seat/grok/auth.json"
  has "$seat force copies validated MCP credentials" 'synthetic' "$home/.local/share/agentboard/seats/$seat/grok/mcp_credentials.json"
done
cp -r "$role" "$tmp/grok-role-before-force-rerun"
grok_inst --force
is "repeated Grok force converges byte-for-byte" diff -r "$tmp/grok-role-before-force-rerun" "$role"
rm "$home/.grok/auth.json" "$home/.grok/mcp_credentials.json"
grok_inst; code=$?
is "credential withdrawal without force succeeds" test "$code" -eq 0
is "withdrawn auth snapshot remains until force" test -e "$home/.local/share/agentboard/seats/analyst/grok/auth.json"
is "withdrawn MCP snapshot remains until force" test -e "$home/.local/share/agentboard/seats/analyst/grok/mcp_credentials.json"
grok_inst --force; code=$?
is "base credential withdrawal force succeeds" test "$code" -eq 0
for seat in analyst coder manager; do
  is "$seat recorded withdrawn auth snapshot removed" test ! -e "$home/.local/share/agentboard/seats/$seat/grok/auth.json"
  is "$seat recorded withdrawn MCP snapshot removed" test ! -e "$home/.local/share/agentboard/seats/$seat/grok/mcp_credentials.json"
done
same "independent auth survives base withdrawal" "$tmp/diverged-auth" "$role/auth.json"
same "independent MCP credential survives base withdrawal" "$tmp/diverged-mcp" "$role/mcp_credentials.json"

# Malformed marked cards and unsafe role paths fail preflight without writes.
newhome
mkdir -p "$home/.grok" "$home/.local/share/agentboard/seats/reviewer/grok"
printf '[ui]\ntheme = "dark"\n# BEGIN AGENTBOARD MCP\n' >"$home/.grok/config.toml"
grok_inst --force; code=$?
is "malformed Grok base markers refused" test "$code" -eq 1
is "malformed Grok base marker writes no homes" test ! -e "$home/.local/share/agentboard/seats/analyst/grok"
newhome
mkdir -p "$home/.grok" "$home/.local/share/agentboard/seats/analyst"
ln -s "$tmp" "$home/.local/share/agentboard/seats/analyst/grok"
grok_inst --force; code=$?
is "symlink Grok role home refused" test "$code" -eq 1
is "symlink Grok role preflight creates no sibling homes" test ! -e "$home/.local/share/agentboard/seats/coder/grok"

# Either malformed credential file refuses all writes and remains private input only.
newhome
mkdir -p "$home/.grok"
printf '{"access_token":"synthetic"}\n' >"$home/.grok/auth.json"
printf '[]\n' >"$home/.grok/mcp_credentials.json"
cp "$home/.grok/auth.json" "$tmp/base-auth"
grok_inst --force; code=$?
is "non-object Grok MCP credential refused" test "$code" -eq 1
is "malformed credential preflight writes no role homes" test ! -e "$home/.local/share/agentboard/seats"
same "malformed credential refusal preserves base auth" "$tmp/base-auth" "$home/.grok/auth.json"
newhome
mkdir -p "$home/.grok"
printf 'not-json' >"$home/.grok/auth.json"
printf '{"server":"synthetic"}\n' >"$home/.grok/mcp_credentials.json"
grok_inst --force; code=$?
is "malformed Grok auth refused" test "$code" -eq 1
is "malformed auth preflight writes no role homes" test ! -e "$home/.local/share/agentboard/seats"

# The legacy config's agentboard table can be replaced only after explicit force.
newhome
mkdir -p "$home/.grok"
printf '[ui]\ntheme = "dark"\nnote = "keep"\n\n[mcp_servers.agentboard]\nurl = "http://wrong"\n\n[mcp_servers.agentboard.env]\nTOKEN = "synthetic"\n\n[mcp_servers.other]\nurl = "http://other"\n' >"$home/.grok/config.toml"
grok_inst; code=$?
is "unmarked Grok agentboard table refused" test "$code" -eq 1
is "unmarked refusal writes no role homes" test ! -e "$home/.local/share/agentboard/seats"
grok_inst --force; code=$?
is "force safely replaces unmarked Grok table" test "$code" -eq 0
has "force preserves unrelated UI setting" 'note = "keep"' "$home/.local/share/agentboard/seats/manager/grok/config.toml"
has "force preserves unrelated MCP table" '[mcp_servers.other]' "$home/.local/share/agentboard/seats/manager/grok/config.toml"
is "force removes unmarked token from role config" bash -c '! grep -q synthetic "$1"' _ "$home/.local/share/agentboard/seats/manager/grok/config.toml"

# --agent grok isolates Grok; --skip-grok never examines a malformed Grok base.
newhome
mkdir -p "$home/.grok" "$home/.codex/rules"
printf '[ui]\ntheme = "dark"\n' >"$home/.grok/config.toml"
printf 'unselected Codex profile' >"$home/.codex/coder.config.toml"
grok_select; code=$?
is "selected Grok skips unrelated malformed Codex input" test "$code" -eq 0
is "selected Grok leaves Codex destination unchanged" test "$(cat "$home/.codex/coder.config.toml")" = 'unselected Codex profile'
rm -rf "$home/.local/share/agentboard/seats/analyst/grok" "$home/.local/share/agentboard/seats/coder/grok" "$home/.local/share/agentboard/seats/manager/grok" "$home/.local/share/agentboard/seats/reviewer/grok"
printf 'not TOML = [' >"$home/.grok/config.toml"
inst --skip-grok --force; code=$?
is "skip Grok succeeds with malformed Grok base" test "$code" -eq 0
is "skip Grok does not publish Grok role homes" test ! -e "$home/.local/share/agentboard/seats/analyst/grok"

# --- Codewhale per-seat homes: own config + MCP card, never ~/.codewhale ---
newhome
mkdir -p "$home/.codex/rules"
inst; code=$?
is "Codewhale install without bases" test "$code" -eq 0
is "absent bases retain generated defaults" python3 -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb")); assert d["reasoning_effort"]=="medium" and d["yolo"] is True' "$home/.local/share/agentboard/seats/coder/codewhale/config.toml"
is "absent MCP base retains generated defaults" python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert list(d["servers"])==["agentboard"] and d["servers"]["agentboard"]["url"].endswith("/mcp/coder")' "$home/.local/share/agentboard/seats/coder/codewhale/mcp.json"
is "no provider leaves auth unconfigured" test ! -e "$home/.local/share/agentboard/seats/coder/codewhale/auth"
if command -v codewhale >/dev/null 2>&1 && [ "${AGENTBOARD_TEST_NO_CODEWHALE:-0}" != 1 ]; then
  for seat in analyst coder manager reviewer; do
    HOME="$home" CODEWHALE_HOME="$home/.local/share/agentboard/seats/$seat/codewhale" codewhale mcp list >"$tmp/mcp-list" 2>&1
    is "$seat codewhale mcp list ends /mcp/$seat" bash -c 'grep -q "/mcp/$1\$" "$2"' _ "$seat" "$tmp/mcp-list"
    HOME="$home" CODEWHALE_HOME="$home/.local/share/agentboard/seats/$seat/codewhale" codewhale config doctor >"$tmp/doctor" 2>&1
    is "$seat codewhale config doctor reports no error" bash -c '! grep -qiE "(^|[^a-z])(error|fail)" "$1"' _ "$tmp/doctor"
  done
  is "Codewhale probes left base config absent" test ! -e "$home/.codewhale/config.toml"
else
  for seat in analyst coder manager reviewer; do
    is "$seat generated MCP URL ends /mcp/$seat" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["servers"]["agentboard"]["url"].endswith("/mcp/"+sys.argv[2])' "$home/.local/share/agentboard/seats/$seat/codewhale/mcp.json" "$seat"
    is "$seat config doctor TOML fallback parses" python3 -c 'import sys,tomllib; tomllib.load(open(sys.argv[1],"rb"))' "$home/.local/share/agentboard/seats/$seat/codewhale/config.toml"
  done
  is "Codewhale probes left base config absent" test ! -e "$home/.codewhale/config.toml"
fi
newhome
mkdir -p "$home/.codex/rules"
mkdir -p "$home/.codewhale"
cat >"$home/.codewhale/mcp.json" <<'EOF'
{"label":"user-config","servers":{"extra":{"command":"helper","args":["--user"]},"agentboard":{"url":"http://wrong"}},"custom":true}
EOF
cat >"$home/.codewhale/config.toml" <<'EOF'
provider = "base-provider"
model = "base-model"
temperature = 0.4
api_key = "secret-config-value"
approval_policy = "auto"
sandbox_mode = "read-only"
sandbox_network_access = false
reasoning_effort = "high"
yolo = false
mcp_config_path = "/wrong/path"

[auth]
token = "nested-secret-value"
EOF
cp "$home/.codewhale/mcp.json" "$tmp/cw-base-mcp"
cp "$home/.codewhale/config.toml" "$tmp/cw-base-config"
make_fake_codewhale
mkdir -p "$home/.codewhale/fake-auth"
printf '%s' 'base-provider-fixture-key' >"$home/.codewhale/fake-auth/base-provider"
printf '%s\n' 'base-provider-fixture-key' >"$tmp/expected-base-key"
printf '%s' 'explicit-provider-fixture-key' >"$home/.codewhale/fake-auth/explicit-provider"
printf '%s\n' 'explicit-provider-fixture-key' >"$tmp/expected-explicit-key"
printf '%s\n' base-provider-fixture-key explicit-provider-fixture-key rotated-base-provider-fixture-key deepseek-fixture-key >"$tmp/fixture-secrets"
cat >"$home/.codewhale/auth-inventory" <<'EOF'
provider config store env route
base-provider yes no no config
explicit-provider no yes no store
unset-provider no no no missing
oauth-provider no yes no oauth (run auth login to register grant)
external-provider no yes no external-consent (awaiting external approval)
EOF
cp "$home/.codewhale/fake-auth/base-provider" "$tmp/base-key-before"
cp "$home/.codewhale/fake-auth/explicit-provider" "$tmp/explicit-key-before"
inst; code=$?
is "codewhale install exit" test "$code" -eq 0
cw=$home/.local/share/agentboard/seats
for seat in analyst coder manager reviewer; do
  c=$cw/$seat/codewhale/config.toml
  has "$seat effort" 'reasoning_effort = "medium"' "$c"
  has "$seat mcp path" "mcp_config_path = \"$cw/$seat/codewhale/mcp.json\"" "$c"
  has "$seat yolo" 'yolo = true' "$c"
  is "$seat MCP preserves base fields and owns agentboard" python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["label"]=="user-config" and d["custom"] and d["servers"]["extra"]=={"command":"helper","args":["--user"]} and d["servers"]["agentboard"]["url"]=="http://127.0.0.1:8765/mcp/"+sys.argv[2]' "$cw/$seat/codewhale/mcp.json" "$seat"
  is "$seat inherits base model settings and seat overrides" python3 -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb")); assert d["provider"]=="base-provider" and d["model"]=="base-model" and d["temperature"]==0.4 and d["reasoning_effort"]=="medium" and d["yolo"] is True and d["mcp_config_path"]==sys.argv[2] and "approval_policy" not in d' "$c" "$cw/$seat/codewhale/mcp.json"
  is "$seat valid TOML" python3 -c 'import sys,tomllib; tomllib.load(open(sys.argv[1],"rb"))' "$c"
done
for seat in analyst coder manager reviewer; do
  HOME="$home" CODEWHALE_HOME="$cw/$seat/codewhale" AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" "$tmp/bin/codewhale" auth print-api-key --provider base-provider >"$tmp/seat-key"
  is "$seat has usable inherited provider key" cmp -s "$tmp/expected-base-key" "$tmp/seat-key"
  HOME="$home" CODEWHALE_HOME="$cw/$seat/codewhale" AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" "$tmp/bin/codewhale" auth print-api-key --provider explicit-provider >"$tmp/seat-key"
  is "$seat has usable inactive provider key" cmp -s "$tmp/expected-explicit-key" "$tmp/seat-key"
  is "$seat does not claim unset provider credentials" test ! -e "$cw/$seat/codewhale/auth/unset-provider.key"
  is "$seat does not claim OAuth grant copied" test ! -e "$cw/$seat/codewhale/auth/oauth-provider.key"
  is "$seat does not claim external-consent route copied" test ! -e "$cw/$seat/codewhale/auth/external-provider.key"
done
same "source auth remains byte-identical after install" "$tmp/base-key-before" "$home/.codewhale/fake-auth/base-provider"
same "other source API key remains byte-identical" "$tmp/explicit-key-before" "$home/.codewhale/fake-auth/explicit-provider"
inst --codewhale-provider explicit-provider
is "explicit provider overrides base provider" python3 -c 'import sys,tomllib; assert tomllib.load(open(sys.argv[1],"rb"))["provider"]=="explicit-provider"' "$cw/coder/codewhale/config.toml"
for seat in analyst coder manager reviewer; do
  HOME="$home" CODEWHALE_HOME="$cw/$seat/codewhale" AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" "$tmp/bin/codewhale" auth print-api-key --provider explicit-provider >"$tmp/seat-key"
  is "$seat has explicit provider key" cmp -s "$tmp/expected-explicit-key" "$tmp/seat-key"
done
inst
is "omitting provider override restores base provider" python3 -c 'import sys,tomllib; assert tomllib.load(open(sys.argv[1],"rb"))["provider"]=="base-provider"' "$cw/coder/codewhale/config.toml"
cp -r "$cw" "$tmp/cw-idempotent-before"
inst
is "auth and generated homes are byte-idempotent" diff -r "$tmp/cw-idempotent-before" "$cw"
printf '%s' 'rotated-base-provider-fixture-key' >"$home/.codewhale/fake-auth/base-provider"
printf '%s\n' 'rotated-base-provider-fixture-key' >"$tmp/expected-rotated-key"
cp "$home/.codewhale/fake-auth/base-provider" "$tmp/rotated-source-key-before"
cp "$home/.codewhale/fake-auth/explicit-provider" "$tmp/explicit-key-before-rotation"
inst
for seat in analyst coder manager reviewer; do
  HOME="$home" CODEWHALE_HOME="$cw/$seat/codewhale" AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" "$tmp/bin/codewhale" auth print-api-key --provider base-provider >"$tmp/seat-key"
  is "$seat receives rotated key" cmp -s "$tmp/expected-rotated-key" "$tmp/seat-key"
  HOME="$home" CODEWHALE_HOME="$cw/$seat/codewhale" AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" "$tmp/bin/codewhale" auth print-api-key --provider explicit-provider >"$tmp/seat-key"
  is "$seat retains inactive key on rotation" cmp -s "$tmp/expected-explicit-key" "$tmp/seat-key"
done
same "rotated source auth remains byte-identical" "$tmp/rotated-source-key-before" "$home/.codewhale/fake-auth/base-provider"
same "other source auth remains byte-identical after rotation" "$tmp/explicit-key-before-rotation" "$home/.codewhale/fake-auth/explicit-provider"
cp -r "$cw" "$tmp/cw-before-inventory-failure"
AGENTBOARD_FAKE_FAIL_LIST=1 inst; code=$?
is "auth inventory failure refuses" test "$code" -eq 1
has "auth inventory failure diagnostic is generic" 'cannot enumerate Codewhale provider auth' "$tmp/err"
is "auth inventory failure leaves all seat files unchanged" diff -r "$tmp/cw-before-inventory-failure" "$cw"
cat >"$home/.codewhale/auth-inventory" <<'EOF'
provider config store env route
base-provider yes maybe no config
EOF
inst; code=$?
is "malformed auth inventory refuses" test "$code" -eq 1
has "malformed auth inventory diagnostic is generic" 'cannot parse Codewhale provider auth inventory' "$tmp/err"
is "malformed auth inventory leaves all seat files unchanged" diff -r "$tmp/cw-before-inventory-failure" "$cw"
cat >"$home/.codewhale/auth-inventory" <<'EOF'
provider config store env route
base-provider yes no no config
explicit-provider no yes no store
unset-provider no no no missing
oauth-provider no yes no oauth (run auth login to register grant)
external-provider no yes no external-consent (awaiting external approval)
EOF
cp -r "$cw" "$tmp/cw-before-missing-source-key"
mv "$home/.codewhale/fake-auth/base-provider" "$tmp/source-key-unavailable"
inst; code=$?
is "missing source API key refuses" test "$code" -eq 1
has "missing source key diagnostic is provider-only" 'cannot retrieve API key for provider base-provider' "$tmp/err"
is "missing source key leaves all seat files unchanged" diff -r "$tmp/cw-before-missing-source-key" "$cw"
mv "$tmp/source-key-unavailable" "$home/.codewhale/fake-auth/base-provider"
cp -r "$cw" "$tmp/cw-before-stage-failure"
AGENTBOARD_FAKE_FAIL_SEAT=reviewer inst; code=$?
is "seat auth staging failure refuses" test "$code" -eq 1
has "seat auth staging failure diagnostic omits secret" 'cannot stage API key for provider base-provider in reviewer home' "$tmp/err"
is "seat auth staging failure leaves all seat files unchanged" diff -r "$tmp/cw-before-stage-failure" "$cw"
cp -r "$cw" "$tmp/cw-before-bad-mcp-base"
printf '{bad json' >"$home/.codewhale/mcp.json"
inst; code=$?
is "malformed MCP base with installed seats refuses" test "$code" -eq 1
is "malformed MCP base leaves all seat files unchanged" diff -r "$tmp/cw-before-bad-mcp-base" "$cw"
cp "$tmp/cw-base-mcp" "$home/.codewhale/mcp.json"
cp -r "$cw" "$tmp/cw-before-bad-config-base"
printf 'provider = [' >"$home/.codewhale/config.toml"
inst; code=$?
is "malformed config base with installed seats refuses" test "$code" -eq 1
is "malformed config base leaves all seat files unchanged" diff -r "$tmp/cw-before-bad-config-base" "$cw"
cp "$tmp/cw-base-config" "$home/.codewhale/config.toml"
same "base MCP remains byte-identical after failure probes" "$tmp/cw-base-mcp" "$home/.codewhale/mcp.json"
same "base config remains byte-identical after failure probes" "$tmp/cw-base-config" "$home/.codewhale/config.toml"
for seat in manager coder; do
  is "$seat has no sandbox keys" bash -c '! grep -q sandbox "$1"' _ "$cw/$seat/codewhale/config.toml"
done
has "analyst sandbox" 'sandbox_mode = "workspace-write"' "$cw/analyst/codewhale/config.toml"
is "analyst no network key" bash -c '! grep -q sandbox_network_access "$1"' _ "$cw/analyst/codewhale/config.toml"
has "reviewer sandbox" 'sandbox_mode = "workspace-write"' "$cw/reviewer/codewhale/config.toml"
has "reviewer network" 'sandbox_network_access = true' "$cw/reviewer/codewhale/config.toml"
same "base MCP config unchanged" "$tmp/cw-base-mcp" "$home/.codewhale/mcp.json"
same "base config unchanged" "$tmp/cw-base-config" "$home/.codewhale/config.toml"
is "no approval_policy in repo templates" bash -c '! grep -rq approval_policy "$1"/seats/codewhale' _ "$repo"
is "credentials stay out of generated config and installer output" bash -c '! grep -q secret-config-value "$1" "$2"' _ "$cw/coder/codewhale/config.toml" "$tmp/out"
is "nested credentials stay out of generated config" bash -c '! grep -q nested-secret-value "$1"' _ "$cw/coder/codewhale/config.toml"
is "fixture keys absent from installer output, generated configs and argv logs" bash -c '! grep -Ff "$1" "$2" "$3" "$4" "$5" "$6" "$7" "$8"' _ "$tmp/fixture-secrets" "$tmp/installer-stdout.log" "$tmp/installer-stderr.log" "$tmp/codewhale-auth.log" "$cw/analyst/codewhale/config.toml" "$cw/coder/codewhale/config.toml" "$cw/manager/codewhale/config.toml" "$cw/reviewer/codewhale/config.toml"
find "$cw" -type f \( -name config.toml -o -name mcp.json \) | sort >"$tmp/cw-files"
find "$cw" -type f | sort >"$tmp/cw-all-files"
for seat in analyst coder manager reviewer; do
  cfg=$cw/$seat/codewhale/config.toml
  is "$seat status hooks parse" python3 -c 'import sys,tomllib; h=tomllib.load(open(sys.argv[1],"rb"))["hooks"]["hooks"]; assert [x["event"] for x in h]==["session_start","message_submit","turn_end"]' "$cfg"
  for st in idle working done; do
    has "$seat reports $st" "]7501;state=$st:app=codewhale" "$cfg"
  done
done
is "twelve generated config and MCP files" test "$(wc -l <"$tmp/cw-files")" -eq 16
is "all exportable auth artifacts preserved" test "$(find "$cw" -type f -path '*/auth/*.key' | wc -l)" -eq 8
is "generated files and auth artifacts have private permissions" bash -c 'while IFS= read -r f; do test "$(stat -c %a "$f")" = 600 || exit 1; done <"$1"' _ "$tmp/cw-all-files"
chmod 644 "$cw/coder/codewhale/auth/base-provider.key"
inst
is "managed auth artifact mode repaired on rerun" test "$(stat -c %a "$cw/coder/codewhale/auth/base-provider.key")" = 600
is "fixture keys absent from output, config and CLI argv log" bash -c '! grep -Ff "$1" "$2" "$3" "$4" "$5"' _ "$tmp/fixture-secrets" "$tmp/out" "$tmp/err" "$tmp/codewhale-auth.log" "$tmp/cw-files"
chmod 644 "$cw/coder/codewhale/mcp.json"
inst
is "managed MCP mode repaired on rerun" test "$(stat -c %a "$cw/coder/codewhale/mcp.json")" = 600
sed -i 's/model = "base-model"/model = "updated-model"/' "$home/.codewhale/config.toml"
python3 -c 'import json,sys; p=sys.argv[1]; d=json.load(open(p)); d["servers"]["extra"]["command"]="updated-helper"; open(p,"w").write(json.dumps(d))' "$home/.codewhale/mcp.json"
inst
is "base changes refresh managed outputs" python3 -c 'import json,sys,tomllib; p=sys.argv[1]+"/coder/codewhale"; m=json.load(open(p+"/mcp.json")); c=tomllib.load(open(p+"/config.toml","rb")); assert m["servers"]["extra"]["command"]=="updated-helper" and c["model"]=="updated-model"' "$cw"
is "generated files carry updated base values" bash -c 'grep -q updated-helper "$1" && grep -q updated-model "$2"' _ "$cw/coder/codewhale/mcp.json" "$cw/coder/codewhale/config.toml"
# a hand-edited config is refused (kept) without --force and replaced with it
cp "$cw/coder/codewhale/config.toml" "$tmp/cw-coder-orig"
printf 'reasoning_effort = "high"\n' >>"$cw/coder/codewhale/config.toml"
cp "$cw/coder/codewhale/config.toml" "$tmp/cw-coder-edited"
inst
has "edited config reported" "unchanged: $cw/coder/codewhale/config.toml (differs" "$tmp/out"
same "edited config kept without force" "$tmp/cw-coder-edited" "$cw/coder/codewhale/config.toml"
inst --force
has "edited config replaced with force" "replaced: $cw/coder/codewhale/config.toml" "$tmp/out"
is "force restores generated config" python3 -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb")); assert d["reasoning_effort"]=="medium"' "$cw/coder/codewhale/config.toml"
# MCP hand edits are protected too, while an explicit force refreshes them.
cp "$cw/coder/codewhale/mcp.json" "$tmp/cw-mcp-orig"
printf '\n ' >>"$cw/coder/codewhale/mcp.json"
cp "$cw/coder/codewhale/mcp.json" "$tmp/cw-mcp-edited"
inst
same "edited MCP file kept without force" "$tmp/cw-mcp-edited" "$cw/coder/codewhale/mcp.json"
inst --force
same "force restores generated MCP file" "$tmp/cw-mcp-orig" "$cw/coder/codewhale/mcp.json"
# Unmarked regular Codewhale files refuse without force and are replaced with it.
printf 'mine = 1\n' >"$cw/manager/codewhale/config.toml"
printf '{"custom":true}' >"$cw/reviewer/codewhale/mcp.json"
cp "$cw/manager/codewhale/config.toml" "$tmp/unmarked-config-before"
cp "$cw/reviewer/codewhale/mcp.json" "$tmp/unmarked-mcp-before"
inst; code=$?
is "unmarked Codewhale files refuse without force" test "$code" -eq 1
has "unmarked config refusal names file" "not generated by agentboard: $cw/manager/codewhale/config.toml" "$tmp/err"
has "unmarked MCP refusal names file" "not generated by agentboard: $cw/reviewer/codewhale/mcp.json" "$tmp/err"
same "unmarked config kept without force" "$tmp/unmarked-config-before" "$cw/manager/codewhale/config.toml"
same "unmarked MCP kept without force" "$tmp/unmarked-mcp-before" "$cw/reviewer/codewhale/mcp.json"
inst --force; code=$?
is "force replaces unmarked regular Codewhale files" test "$code" -eq 0
has "force replaced config with generated marker" '# agentboardGenerated' "$cw/manager/codewhale/config.toml"
is "force replaced MCP with seat card" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["servers"]["agentboard"]["url"].endswith("/mcp/reviewer")' "$cw/reviewer/codewhale/mcp.json"

# Exact Codewhale seat-file symlinks require force and are replaced, never followed.
newhome
mkdir -p "$home/.codex/rules"
inst
cw=$home/.local/share/agentboard/seats
printf 'outside config' >"$tmp/foreign-codewhale-config"
printf 'outside MCP' >"$tmp/foreign-codewhale-mcp"
rm "$cw/manager/codewhale/config.toml"
ln -s "$tmp/foreign-codewhale-config" "$cw/manager/codewhale/config.toml"
rm "$cw/coder/codewhale/mcp.json"
ln -s "$tmp/foreign-codewhale-mcp" "$cw/coder/codewhale/mcp.json"
inst; code=$?
is "Codewhale foreign seat links refuse without force" test "$code" -eq 1
is "Codewhale config link preserved without force" test -L "$cw/manager/codewhale/config.toml"
is "Codewhale MCP link preserved without force" test -L "$cw/coder/codewhale/mcp.json"
mkdir -p "$home/.codewhale/fake-auth"
printf 'stage-link-fixture-key' >"$home/.codewhale/fake-auth/deepseek"
printf 'provider config store env route\ndeepseek yes no no config\n' >"$home/.codewhale/auth-inventory"
inst --force; code=$?
is "force replaces Codewhale links with regular private files" test "$code" -eq 0 -a ! -L "$cw/manager/codewhale/config.toml" -a ! -L "$cw/coder/codewhale/mcp.json" -a "$(stat -c %a "$cw/manager/codewhale/config.toml")" = 600 -a "$(stat -c %a "$cw/coder/codewhale/mcp.json")" = 600
is "Codewhale link targets untouched" test "$(cat "$tmp/foreign-codewhale-config")" = 'outside config' -a "$(cat "$tmp/foreign-codewhale-mcp")" = 'outside MCP'

# Directories remain unsafe and are rejected during preflight.
newhome
mkdir -p "$home/.codex/rules"
inst
cw=$home/.local/share/agentboard/seats
rm "$cw/coder/codewhale/mcp.json"
mkdir "$cw/coder/codewhale/mcp.json"
inst --force; code=$?
is "Codewhale directory refuses under force" test "$code" -eq 1
is "Codewhale directory survives refusal" test -d "$cw/coder/codewhale/mcp.json"

# Malformed bases refuse during preflight, before any installed files change.
newhome
mkdir -p "$home/.codex/rules" "$home/.codewhale"
printf '{bad json' >"$home/.codewhale/mcp.json"
inst; code=$?
is "malformed Codewhale base refused" test "$code" -eq 1
has "malformed Codewhale base diagnosed" 'invalid JSON in' "$tmp/err"
is "malformed base wrote no seat output" test ! -e "$home/.local/share/agentboard/seats"
is "malformed base wrote no Claude outputs" bash -c 'test -z "$(find "$1/.claude" -type f -print -quit)"' _ "$home"
newhome
mkdir -p "$home/.codex/rules" "$home/.codewhale"
printf 'provider = [' >"$home/.codewhale/config.toml"
inst; code=$?
is "malformed Codewhale TOML refused" test "$code" -eq 1
has "malformed Codewhale TOML diagnosed" 'invalid TOML in' "$tmp/err"
is "malformed TOML wrote no seat output" test ! -e "$home/.local/share/agentboard/seats"

# --codewhale-provider writes only the provider name; a bad name is a usage error
newhome
mkdir -p "$home/.codex/rules"
mkdir -p "$home/.codewhale/fake-auth"
printf '%s' 'deepseek-fixture-key' >"$home/.codewhale/fake-auth/deepseek"
printf 'provider config store env route\ndeepseek yes no no config\n' >"$home/.codewhale/auth-inventory"
printf '%s\n' 'deepseek-fixture-key' >"$tmp/expected-deepseek-key"
printf '%s\n' deepseek-fixture-key >>"$tmp/fixture-secrets"
inst --codewhale-provider deepseek; code=$?
is "provider install exit" test "$code" -eq 0
for seat in analyst coder manager reviewer; do
  has "$seat provider line" 'provider = "deepseek"' "$home/.local/share/agentboard/seats/$seat/codewhale/config.toml"
  is "$seat provider config valid TOML" python3 -c 'import sys,tomllib; c=tomllib.load(open(sys.argv[1],"rb")); assert c["provider"]=="deepseek" and len(c["hooks"]["hooks"])==3' "$home/.local/share/agentboard/seats/$seat/codewhale/config.toml"
  HOME="$home" CODEWHALE_HOME="$home/.local/share/agentboard/seats/$seat/codewhale" AGENTBOARD_FAKE_AUTH_DIR="$home/.codewhale/fake-auth" \
    AGENTBOARD_FAKE_AUTH_LOG="$tmp/codewhale-auth.log" "$tmp/bin/codewhale" auth print-api-key --provider deepseek >"$tmp/seat-key"
  is "$seat receives explicit provider key" cmp -s "$tmp/expected-deepseek-key" "$tmp/seat-key"
done
is "all fixture API keys absent from generated config files" bash -c '! grep -Ff "$1" "$2" "$3" "$4" "$5"' _ "$tmp/fixture-secrets" "$home/.local/share/agentboard/seats/analyst/codewhale/config.toml" "$home/.local/share/agentboard/seats/coder/codewhale/config.toml" "$home/.local/share/agentboard/seats/manager/codewhale/config.toml" "$home/.local/share/agentboard/seats/reviewer/codewhale/config.toml"
is "explicit provider auth artifact is private" test "$(stat -c %a "$home/.local/share/agentboard/seats/coder/codewhale/auth/deepseek.key")" = 600
is "explicit provider key absent from config" bash -c '! grep -q deepseek-fixture-key "$1"' _ "$home/.local/share/agentboard/seats/coder/codewhale/config.toml"
is "all fixture keys absent from installer output and argv logs" bash -c '! grep -Ff "$1" "$2" "$3" "$4"' _ "$tmp/fixture-secrets" "$tmp/installer-stdout.log" "$tmp/installer-stderr.log" "$tmp/codewhale-auth.log"
newhome
inst --codewhale-provider 'x"; yolo = false'; code=$?
is "bad provider name usage error" test "$code" -eq 2
is "bad provider writes nothing" test ! -e "$home/.local/share/agentboard"
inst --grok-only; code=$?
is "grok-only writes no Codewhale home" test ! -e "$home/.local/share/agentboard/seats/coder/codewhale"

# Root entrypoint and selective client preflight/write boundaries.
newhome
mkdir -p "$home/.codex/rules" "$home/.grok" "$home/.codewhale"
printf '{bad json' >"$home/.claude.json"
printf '{bad grok' >"$home/.grok/config.toml"
printf '{bad codewhale' >"$home/.codewhale/mcp.json"
root_inst --agent codex; code=$?
is "root codex singleton succeeds despite malformed unselected inputs" test "$code" -eq 0
is "codex singleton installs codex profile" test -L "$home/.codex/coder.config.toml"
is "codex singleton skips Claude files" test ! -e "$home/.claude/coder-mcp.json"
is "codex singleton skips Codewhale files" test ! -e "$home/.local/share/agentboard/seats/coder/codewhale"
is "codex singleton skips Grok config" test "$(cat "$home/.grok/config.toml")" = '{bad grok'
is "codex singleton skips shared launcher and unit" test ! -e "$home/.local/bin/start-agentboard-manager" -a ! -e "$home/.config/systemd/user/agentboard.service"

newhome
mkdir -p "$home/.codex/rules" "$home/.grok" "$home/.codewhale"
printf '{}' >"$home/.claude.json"
printf '{bad grok' >"$home/.grok/config.toml"
printf '{bad codewhale' >"$home/.codewhale/mcp.json"
root_inst --agent codex,claude; code=$?
is "root codex and Claude pair ignores malformed unselected clients" test "$code" -eq 0
is "pair writes Claude output" test -f "$home/.claude/coder-mcp.json"
is "pair writes Codex output" test -L "$home/.codex/coder.config.toml"
is "pair skips Codewhale output" test ! -e "$home/.local/share/agentboard/seats/coder/codewhale"
is "pair leaves Grok bytes alone" test "$(cat "$home/.grok/config.toml")" = '{bad grok'
is "pair leaves launcher and unit alone" test ! -e "$home/.local/bin/start-agentboard-manager" -a ! -e "$home/.config/systemd/user/agentboard.service"

newhome
mkdir -p "$home/.codex/rules"
root_inst --agent codex,codewhale; code=$?
is "root codex and Codewhale pair succeeds" test "$code" -eq 0
is "Codewhale pair writes Codex profile" test -L "$home/.codex/coder.config.toml"
is "Codewhale pair writes selected Codewhale home" test -f "$home/.local/share/agentboard/seats/coder/codewhale/config.toml" -a -f "$home/.local/share/agentboard/seats/coder/codewhale/mcp.json"
is "Codewhale pair skips Claude files" test ! -e "$home/.claude/coder-mcp.json"
is "Codewhale pair skips Grok" test ! -e "$home/.grok/config.toml"
is "Codewhale pair skips shared launcher and unit" test ! -e "$home/.local/bin/start-agentboard-manager" -a ! -e "$home/.config/systemd/user/agentboard.service"

newhome
mkdir -p "$home/.grok" "$home/.codewhale"
printf '{bad json' >"$home/.claude.json"
printf '{bad codewhale' >"$home/.codewhale/mcp.json"
root_inst --agent grok; code=$?
is "root Grok singleton succeeds despite malformed unselected inputs" test "$code" -eq 0
is "Grok singleton installs manager role config" grep -qF 'url = "http://127.0.0.1:8765/mcp/manager"' "$home/.local/share/agentboard/seats/manager/grok/config.toml"
is "Grok singleton skips Codex profiles" test ! -e "$home/.codex/coder.config.toml"
is "Grok singleton skips Claude outputs" test ! -e "$home/.claude/coder-mcp.json"
is "Grok singleton skips Codewhale outputs" test ! -e "$home/.local/share/agentboard/seats/coder/codewhale"
is "Grok singleton skips launcher and unit" test ! -e "$home/.local/bin/start-agentboard-manager" -a ! -e "$home/.config/systemd/user/agentboard.service"

newhome
mkdir -p "$home/.codex/rules" "$home/.grok"
printf '[broken\n' >"$home/.grok/config.toml"
cp "$home/.grok/config.toml" "$tmp/selected-grok-before"
root_inst --agent codex,grok; code=$?
is "selected malformed Grok preflight refuses before writes" test "$code" -eq 1
has "selected malformed Grok preflight diagnoses TOML" 'invalid TOML' "$tmp/err"
same "selected malformed Grok preflight preserves config" "$tmp/selected-grok-before" "$home/.grok/config.toml"
is "selected malformed Grok preflight writes no Codex output" test ! -e "$home/.codex/coder.config.toml" -a ! -e "$home/.codex/rules/default.rules"

newhome
mkdir -p "$home/.codex/rules" "$home/.codewhale"
printf '{bad json' >"$home/.codewhale/mcp.json"
cp "$home/.codewhale/mcp.json" "$tmp/selected-codewhale-before"
root_inst --agent codex,codewhale; code=$?
is "selected malformed Codewhale preflight refuses before writes" test "$code" -eq 1
has "selected malformed Codewhale preflight diagnoses JSON" 'invalid JSON in' "$tmp/err"
same "selected malformed Codewhale preflight preserves base" "$tmp/selected-codewhale-before" "$home/.codewhale/mcp.json"
is "selected malformed Codewhale preflight writes no Codex output" test ! -e "$home/.codex/coder.config.toml" -a ! -e "$home/.codex/rules/default.rules"
is "selected malformed Codewhale preflight writes no seat home" test ! -e "$home/.local/share/agentboard/seats"

newhome
mkdir -p "$home/.codex/rules"
printf '{bad json' >"$home/.claude.json"
root_inst --agent claude; code=$?
is "selected malformed Claude input refuses" test "$code" -eq 1
is "selected malformed Claude input writes no output" test ! -e "$home/.claude/coder-mcp.json" -a ! -e "$home/.codex/coder.config.toml" -a ! -e "$home/.local/bin/start-agentboard-manager"
is "selected malformed Claude input writes no role home" test ! -e "$home/.local/share/agentboard/seats"

newhome
mkdir -p "$home/.codex/rules" "$home/.grok" "$home/.codewhale"
printf '[malformed' >"$home/.grok/config.toml"
printf '{malformed' >"$home/.codewhale/mcp.json"
root_inst --agent claude; code=$?
is "selected Claude succeeds with malformed unselected clients" test "$code" -eq 0
is "selected Claude publishes all four role homes" test -f "$home/.local/share/agentboard/seats/analyst/claude/mcp.json" -a -f "$home/.local/share/agentboard/seats/coder/claude/mcp.json" -a -f "$home/.local/share/agentboard/seats/manager/claude/mcp.json" -a -f "$home/.local/share/agentboard/seats/reviewer/claude/mcp.json"
is "selected Claude leaves other clients and shared installs untouched" test ! -e "$home/.codex/coder.config.toml" -a ! -e "$home/.local/share/agentboard/seats/coder/codex" -a ! -e "$home/.local/share/agentboard/seats/coder/codewhale" -a "$(cat "$home/.grok/config.toml")" = '[malformed' -a ! -e "$home/.local/bin/start-agentboard-manager" -a ! -e "$home/.config/systemd/user/agentboard.service"

for input in settings credentials; do
  newhome
  mkdir -p "$home/.claude"
  if [[ $input == settings ]]; then printf '{bad json' >"$home/.claude/settings.json"; else printf '{bad json' >"$home/.claude/.credentials.json"; fi
  root_inst --agent claude --force; code=$?
  is "malformed Claude $input refuses before publication" test "$code" -eq 1 -a ! -e "$home/.local/share/agentboard/seats" -a ! -e "$home/.claude/manager-mcp.json"
done

newhome
mkdir -p "$home/.local/share/agentboard/seats/manager"
ln -s "$tmp" "$home/.local/share/agentboard/seats/manager/claude"
root_inst --agent claude --force; code=$?
is "symlinked Claude role home refuses under force" test "$code" -eq 1
is "symlinked Claude role home target untouched" test ! -e "$tmp/settings.json" -a ! -e "$tmp/mcp.json" -a ! -e "$home/.local/share/agentboard/seats/analyst/claude"

for selector in '' codex, codex,,claude codex,codex Codex mystery; do
  newhome
  root_inst --agent "$selector"; code=$?
  is "invalid --agent $selector exits usage 2 before writes" test "$code" -eq 2 -a ! -e "$home/.codex/coder.config.toml" -a ! -e "$home/.claude/coder-mcp.json"
done
newhome
root_inst --agent codex --agent claude; code=$?
is "repeated --agent exits usage 2 before writes" test "$code" -eq 2 -a ! -e "$home/.codex/coder.config.toml"
newhome
root_inst --agent codex --skip-grok; code=$?
is "selector with legacy Grok flag exits usage 2" test "$code" -eq 2 -a ! -e "$home/.codex/coder.config.toml"
newhome
root_inst --agent claude --codewhale-provider custom; code=$?
is "provider without selected Codewhale exits usage 2" test "$code" -eq 2 -a ! -e "$home/.claude/coder-mcp.json"
newhome
root_inst --codewhale-provider custom --skip-grok; code=$?
is "provider outside default full install exits usage 2" test "$code" -eq 2 -a ! -e "$home/.codex/coder.config.toml"
newhome
root_inst --agent codewhale --codewhale-provider custom; code=$?
is "provider override is valid for selected Codewhale" test "$code" -eq 0 -a -f "$home/.local/share/agentboard/seats/coder/codewhale/config.toml"
is "selected Codewhale provider is applied" grep -qF 'provider = "custom"' "$home/.local/share/agentboard/seats/coder/codewhale/config.toml"

# Legacy Codewhale homes are copied into role-first destinations without mutation.
newhome
mkdir -p "$home/.codex/rules" "$home/.codewhale/fake-auth"
make_fake_codewhale
printf '%s' 'migration-provider-fixture-key' >"$home/.codewhale/fake-auth/migration-provider"
printf 'provider config store env route\nmigration-provider yes no no config\n' >"$home/.codewhale/auth-inventory"
legacy=$home/.local/share/agentboard/codewhale
for seat in analyst coder manager reviewer; do
  mkdir -p "$legacy/$seat/auth"
  printf 'legacy user setting\n' >"$legacy/$seat/preferences.txt"
  printf 'legacy auth artifact\n' >"$legacy/$seat/auth/session.data"
  printf 'provider = "legacy-provider"\n' >"$legacy/$seat/config.toml"
  printf '{"servers":{"old":{"command":"kept"}}}\n' >"$legacy/$seat/mcp.json"
done
cp -R "$legacy" "$tmp/legacy-before"
inst --agent codewhale; code=$?
is "legacy Codewhale migration succeeds" test "$code" -eq 0
for seat in analyst coder manager reviewer; do
  dest=$home/.local/share/agentboard/seats/$seat/codewhale
  is "$seat old home retained byte-identically" diff -r "$tmp/legacy-before/$seat" "$legacy/$seat"
  is "$seat unrelated file migrated" test "$(cat "$dest/preferences.txt")" = 'legacy user setting'
  is "$seat old auth artifact migrated privately" test "$(cat "$dest/auth/session.data")" = 'legacy auth artifact' -a "$(stat -c %a "$dest/auth/session.data")" = 600
  is "$seat config regenerated from base and template" grep -qF "mcp_config_path = \"$dest/mcp.json\"" "$dest/config.toml"
  is "$seat MCP regenerated for its actor" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))["servers"]["agentboard"]["url"].endswith("/mcp/"+sys.argv[2])' "$dest/mcp.json" "$seat"
  is "$seat migrated exportable API key retained" test -f "$dest/auth/migration-provider.key"
done
is "legacy migration did not create old-home links" bash -c '! find "$1" -type l | grep -q .' _ "$legacy"
is "legacy migration outputs are private" test "$(stat -c %a "$home/.local/share/agentboard/seats/coder/codewhale")" = 700

# Unsafe legacy entries refuse before any role-first destination is published.
newhome
mkdir -p "$home/.codex/rules" "$home/.local/share/agentboard/codewhale/reviewer"
ln -s "$tmp" "$home/.local/share/agentboard/codewhale/reviewer/outside"
inst --agent codewhale; code=$?
is "unsafe legacy symlink refuses migration" test "$code" -eq 1
has "unsafe legacy migration diagnostic" 'unsupported entry in Codewhale seat home' "$tmp/err"
is "unsafe legacy migration publishes no destination" test ! -e "$home/.local/share/agentboard/seats"
is "unsafe legacy migration preserves old symlink" test -L "$home/.local/share/agentboard/codewhale/reviewer/outside"

# An existing canonical destination refreshes in place and leaves both its
# unrelated files and any legacy source untouched.
newhome
mkdir -p "$home/.codex/rules" "$home/.local/share/agentboard/seats/coder/codewhale" "$home/.local/share/agentboard/codewhale/coder"
printf 'canonical unrelated content\n' >"$home/.local/share/agentboard/seats/coder/codewhale/keep.txt"
printf 'legacy stays untouched\n' >"$home/.local/share/agentboard/codewhale/coder/keep.txt"
inst --agent codewhale; code=$?
is "existing role-first home refresh succeeds" test "$code" -eq 0
is "existing role-first unrelated file preserved" test "$(cat "$home/.local/share/agentboard/seats/coder/codewhale/keep.txt")" = 'canonical unrelated content'
is "existing destination leaves legacy source untouched" test "$(cat "$home/.local/share/agentboard/codewhale/coder/keep.txt")" = 'legacy stays untouched'

newhome
mkdir -p "$home/.codex/rules"
printf 'preserve unselected destination' >"$home/.claude/coder-mcp.json"
root_inst --agent codex --force; code=$?
is "force selected Codex install succeeds" test "$code" -eq 0
is "force selected Codex preserves Claude destination" test "$(cat "$home/.claude/coder-mcp.json")" = 'preserve unselected destination'

has "team manual documents Grok role homes for each seat" 'GROK_HOME="$HOME/.local/share/agentboard/seats/coder/grok"' "$repo/skills/agentboard-team/SKILL.md"
has "manager skill documents herdr Grok home and read-only policy" 'GROK_HOME="$HOME/.local/share/agentboard/seats/analyst/grok" herdr agent start analyst' "$repo/skills/agentboard-manager/SKILL.md"
has "seat README documents reviewer Grok home" 'GROK_HOME="$HOME/.local/share/agentboard/seats/reviewer/grok"' "$repo/seats/README.md"

echo "$((n - fail))/$n passed (seats installer)"
[ "$fail" -eq 0 ]
