#!/usr/bin/env bash
# ./install.py against isolated homes (cargo is a stub; nothing is built): skills, launcher links, one
# global agentboard MCP entry per client, and old role homes named as leftovers, never touched.
set -u
cd "$(dirname "$0")/.."
repo=$(pwd)
launcher=$repo/seats/agentboard-manager
unit=$repo/seats/systemd/agentboard.service
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
# Every real client CLI writes under the temp $HOME, never a home the caller runs in.
unset CLAUDE_CONFIG_DIR GROK_HOME CODEWHALE_HOME CODEX_HOME
real_codex=$(command -v codex || true)
real_codewhale=$(command -v codewhale || true)
real_claude=$(command -v claude || true)
real_grok=$(command -v grok || true)
real_agy=$(command -v agy || true)
n=0 fail=0
ok() { n=$((n + 1)); }
bad() { n=$((n + 1)); fail=$((fail + 1)); echo "FAIL: $*" >&2; }
is() { local name=$1; shift; if "$@"; then ok; else bad "$name"; fi; }
has() { local name=$1 needle=$2 file=$3; if grep -qF -- "$needle" "$file"; then ok; else bad "$name: missing '$needle' in $(cat "$file")"; fi; }

cases=0
newhome() { # newhome: a clean HOME
  cases=$((cases + 1))
  home=$tmp/h$cases
  mkdir -p "$home"
}
inst() { # inst [args]: run the installer; stdout in $tmp/out, stderr in $tmp/err
  HOME="$home" PATH="$tmp/bin:$PATH" ./install.py "$@" >"$tmp/out" 2>"$tmp/err"
}

# Client stubs ignore `mcp` calls (logging them), except in the one case that sets
# real_<cli>, where each stub hands the call to the real binary.
mkdir -p "$tmp/bin"
for cli in claude codex grok codewhale agy; do
  cat >"$tmp/bin/$cli" <<SH
#!/usr/bin/env bash
var=real_$cli
[ -n "\${!var:-}" ] && exec "\${!var}" "\$@"
echo "$cli \$*" >>"\${STUB_MCP_LOG:-/dev/null}"
case "\$2" in
  remove) [ -z "\${STUB_MCP_REMOVE_FAIL:-}" ] || { echo "\$STUB_MCP_REMOVE_FAIL" >&2; exit 1; } ;;
  add|add-json) [ -z "\${STUB_MCP_ADD_FAIL:-}" ] || { echo "\$STUB_MCP_ADD_FAIL" >&2; exit 1; } ;;
esac
exit 0
SH
  chmod 700 "$tmp/bin/$cli"
done
# cargo is always this stub: it logs its argv and exits $STUB_CARGO_EXIT (default 0).
cat >"$tmp/bin/cargo" <<'SH'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$STUB_CARGO_LOG"
exit "${STUB_CARGO_EXIT:-0}"
SH
chmod 700 "$tmp/bin/cargo"
export STUB_CARGO_LOG="$tmp/cargo.log"

# --- retired files are gone ---------------------------------------------------
is "per-seat cards and profiles are deleted" bash -c '! ls seats/claude seats/codex >/dev/null 2>&1'
is "role-home code is deleted from the installer" bash -c '! grep -qE "CODEX_SEAT_MARK|CLAUDE_SEAT_MARK|GROK_HOME_ROOT" install.py'

# --- fresh full install: skills, launcher, unit, and the exact global commands ---
newhome
: >"$tmp/mcp.log"
STUB_MCP_LOG="$tmp/mcp.log" inst --skip-binary; code=$?
is "fresh install exit" test "$code" -eq 0
is "manager launcher linked" test "$(readlink "$home/.local/bin/agentboard-manager")" = "$launcher"
is "user unit linked" test "$(readlink "$home/.config/systemd/user/agentboard.service")" = "$unit"
has "unit restarts on failure" 'Restart=on-failure' "$home/.config/systemd/user/agentboard.service"
has "unit prevents exit one restart" 'RestartPreventExitStatus=1' "$home/.config/systemd/user/agentboard.service"
for s in analyst counter-analyst coder manager reviewer team; do
  is "skill agentboard-$s linked" test "$(readlink "$home/.agents/skills/agentboard-$s")" = "$repo/skills/agentboard-$s"
done
has "codex add argv" "codex mcp add agentboard --url http://127.0.0.1:8765/mcp --bearer-token-env-var AGENTBOARD_ACTOR" "$tmp/mcp.log"
has "claude remove argv" "claude mcp remove -s user agentboard" "$tmp/mcp.log"
has "claude add argv" 'claude mcp add-json -s user agentboard {"type": "http", "url": "http://127.0.0.1:8765/mcp", "headers": {"Authorization": "Bearer ${AGENTBOARD_ACTOR}"}}' "$tmp/mcp.log"
has "grok add argv" 'grok mcp add -s user --transport http agentboard http://127.0.0.1:8765/mcp -H Authorization: Bearer ${AGENTBOARD_ACTOR}' "$tmp/mcp.log"
has "codewhale add argv" "codewhale mcp add agentboard --url http://127.0.0.1:8765/mcp --bearer-token-env-var AGENTBOARD_ACTOR" "$tmp/mcp.log"
has "agy remove argv" "agy mcp remove agentboard" "$tmp/mcp.log"
has "agy add argv names the shim" "agy mcp add agentboard $repo/seats/agy/agentboard-stdio" "$tmp/mcp.log"
is "agy skills link through the canonical names" test "$(readlink "$home/.gemini/config/skills/agentboard-analyst")" = "$home/.agents/skills/agentboard-analyst" -a "$(readlink -f "$home/.gemini/config/skills/agentboard-analyst")" = "$repo/skills/agentboard-analyst"
is "claude skills link through the canonical names" test "$(readlink "$home/.claude/skills/agentboard-counter-analyst")" = "$home/.agents/skills/agentboard-counter-analyst" -a "$(readlink -f "$home/.claude/skills/agentboard-counter-analyst")" = "$repo/skills/agentboard-counter-analyst"
is "no role home is created" test ! -e "$home/.local/share/agentboard"
is "fresh install names no leftovers" bash -c '! grep -q "^note: leftover" "$1"' _ "$tmp/out"
has "done line" "done: agentboard installed" "$tmp/out"

# --- old role homes, legacy homes and cards: named as leftovers, byte-identical ---
newhome
seats=$home/.local/share/agentboard/seats
for seat in analyst coder manager reviewer; do
  for cli in codex claude grok codewhale; do
    mkdir -p "$seats/$seat/$cli"
    printf 'session of %s %s\n' "$seat" "$cli" >"$seats/$seat/$cli/session.log"
    printf 'credential\n' >"$seats/$seat/$cli/auth.json"
  done
done
mkdir -p "$home/.local/share/agentboard/codewhale/coder" "$home/.codex" "$home/.claude" "$home/.local/bin"
printf 'legacy\n' >"$home/.local/share/agentboard/codewhale/coder/keep.txt"
ln -s "$repo/seats/codex/coder.config.toml" "$home/.codex/coder.config.toml"
printf '{"agentboardGenerated":"x"}\n' >"$home/.claude/coder-mcp.json"
ln -s "$repo/seats/start-agentboard-manager" "$home/.local/bin/start-agentboard-manager"
before=$(cd "$home/.local/share/agentboard" && find . -type f -print0 | sort -z | xargs -0 cksum | cksum)
list_before=$(cd "$home/.local/share/agentboard" && find . | sort)
for run in first second; do
  inst --skip-binary; code=$?
  is "$run run with old homes exits 0" test "$code" -eq 0
  is "$run run leaves every home byte-identical" test "$(cd "$home/.local/share/agentboard" && find . -type f -print0 | sort -z | xargs -0 cksum | cksum)" = "$before"
  is "$run run adds nothing under the seats directory" test "$(cd "$home/.local/share/agentboard" && find . | sort)" = "$list_before"
  for seat in analyst coder manager reviewer; do
    for cli in codex claude grok codewhale; do
      has "$run run names $seat $cli home" "note: leftover from an older installer, no longer used; delete it by hand: $seats/$seat/$cli" "$tmp/out"
    done
  done
  has "$run run names the legacy Codewhale home" "$home/.local/share/agentboard/codewhale/coder" "$tmp/out"
  has "$run run names the old Codex profile link" "delete it by hand: $home/.codex/coder.config.toml" "$tmp/out"
  has "$run run names the old Claude card" "delete it by hand: $home/.claude/coder-mcp.json" "$tmp/out"
  has "$run run names the old launcher link" "delete it by hand: $home/.local/bin/start-agentboard-manager" "$tmp/out"
done
is "old Codex profile link kept" test -L "$home/.codex/coder.config.toml"

# --- manager launcher and user unit follow the foreign-link rules ---------------
newhome
mkdir -p "$home/.local/bin"
ln -s /old/checkout/agentboard-manager "$home/.local/bin/agentboard-manager"
inst --skip-binary; code=$?
is "foreign launcher link refused" test "$code" -eq 1
has "foreign launcher link diagnosed" 'foreign link:' "$tmp/err"
is "foreign launcher link preserved" test "$(readlink "$home/.local/bin/agentboard-manager")" = /old/checkout/agentboard-manager
is "foreign launcher refusal links no unit" test ! -e "$home/.config/systemd/user/agentboard.service"
inst --skip-binary --force
is "force replaces launcher link" test "$(readlink "$home/.local/bin/agentboard-manager")" = "$launcher"
newhome
mkdir -p "$home/.config/systemd/user"
ln -s /old/checkout/agentboard.service "$home/.config/systemd/user/agentboard.service"
inst --skip-binary; code=$?
is "foreign unit link refused" test "$code" -eq 1
has "foreign unit link diagnosed" 'foreign link:' "$tmp/err"
is "foreign unit refusal registers nothing" test ! -e "$home/.agents"
inst --skip-binary --force
is "force replaces unit link" test "$(readlink "$home/.config/systemd/user/agentboard.service")" = "$unit"

# --- skills: one canonical link per repo skill in ~/.agents/skills ----------------
skill_names="agentboard-analyst agentboard-counter-analyst agentboard-coder agentboard-manager agentboard-reviewer agentboard-team"
for agent in codex grok codewhale claude agy; do
  newhome
  inst --agent "$agent" --skip-binary; code=$?
  is "--agent $agent install succeeds" test "$code" -eq 0
  is "--agent $agent links all six canonical skills" bash -c 'for s in $2; do test "$(readlink "$1/.agents/skills/$s")" = "$3/skills/$s" || exit 1; done' _ "$home" "$skill_names" "$repo"
  has "--agent $agent reports a linked skill" "linked: $home/.agents/skills/agentboard-team -> $repo/skills/agentboard-team" "$tmp/out"
  if [ "$agent" = claude ]; then
    is "--agent claude links the skills into ~/.claude" test "$(readlink "$home/.claude/skills/agentboard-team")" = "$home/.agents/skills/agentboard-team"
  else
    is "--agent $agent writes no skill alias into ~/.claude" test ! -e "$home/.claude/skills"
  fi
  is "--agent $agent leaves launcher and unit alone" test ! -e "$home/.local/bin/agentboard-manager" -a ! -e "$home/.config/systemd/user/agentboard.service"
done
inst --agent claude --skip-binary
is "rerun leaves canonical skills alone" bash -c '! grep -q "^linked: [^ ]*/.agents/skills/" "$1"' _ "$tmp/out"
printf 'unrelated\n' >"$tmp/other-skill"
ln -s "$tmp/other-skill" "$home/.agents/skills/someone-else"
ln -sfn /old/checkout/skills/agentboard-team "$home/.agents/skills/agentboard-team"
rm "$home/.agents/skills/agentboard-coder"
printf 'mine\n' >"$home/.agents/skills/agentboard-coder"
inst --agent codex --skip-binary; code=$?
is "foreign canonical skill entries refuse without force" test "$code" -eq 1
has "foreign skill link named" "exists: $home/.agents/skills/agentboard-team (use --force)" "$tmp/err"
has "foreign skill file named" "exists: $home/.agents/skills/agentboard-coder (use --force)" "$tmp/err"
is "refusal keeps foreign skill link" test "$(readlink "$home/.agents/skills/agentboard-team")" = /old/checkout/skills/agentboard-team
inst --agent codex --skip-binary --force; code=$?
is "force replaces foreign skill entries" test "$code" -eq 0 -a "$(readlink "$home/.agents/skills/agentboard-team")" = "$repo/skills/agentboard-team" -a "$(readlink "$home/.agents/skills/agentboard-coder")" = "$repo/skills/agentboard-coder"
is "unrelated skills left alone" test "$(readlink "$home/.agents/skills/someone-else")" = "$tmp/other-skill"
rm "$home/.agents/skills/agentboard-analyst"
mkdir -p "$home/.agents/skills/agentboard-analyst"
printf 'keep\n' >"$home/.agents/skills/agentboard-analyst/notes"
inst --agent codex --skip-binary --force; code=$?
is "real skill directory refuses even under force" test "$code" -eq 1
has "real skill directory named" "real directory at skill destination: $home/.agents/skills/agentboard-analyst" "$tmp/err"
is "real skill directory contents kept" test "$(cat "$home/.agents/skills/agentboard-analyst/notes")" = keep
newhome
printf 'not a dir' >"$home/.agents"
inst --agent grok --skip-binary; code=$?
is "non-directory ~/.agents refuses" test "$code" -eq 1
has "non-directory ~/.agents named in preflight" "invalid skills directory: $home/.agents" "$tmp/err"
is "non-directory ~/.agents refusal is not a crash" bash -c '! grep -q Traceback "$1"' _ "$tmp/err"

# --- the binary: cargo install after preflight, before any registration -----------
newhome
: >"$STUB_CARGO_LOG"
inst --agent grok; code=$?
is "default install builds the binary" test "$code" -eq 0 -a "$(cat "$STUB_CARGO_LOG")" = "install --path $repo --locked"
has "build is announced" "building: cargo install --path $repo --locked" "$tmp/out"
newhome
: >"$STUB_CARGO_LOG"
inst --agent grok --skip-binary; code=$?
is "--skip-binary never runs cargo" test "$code" -eq 0 -a ! -s "$STUB_CARGO_LOG"
newhome
: >"$STUB_CARGO_LOG"; : >"$tmp/mcp.log"
STUB_MCP_LOG="$tmp/mcp.log" STUB_CARGO_EXIT=101 inst; code=$?
is "failed build exits 1" test "$code" -eq 1
has "failed build says nothing else was written" "cargo install failed (exit 101); nothing else was written" "$tmp/err"
is "failed build writes no skill, launcher or MCP entry" test ! -e "$home/.agents" -a ! -e "$home/.local/bin/agentboard-manager" -a ! -s "$tmp/mcp.log"
newhome
mkdir -p "$home/.local/bin"
ln -s /old/checkout/agentboard-manager "$home/.local/bin/agentboard-manager"
: >"$STUB_CARGO_LOG"
inst; code=$?
is "preflight refusal never builds" test "$code" -eq 1 -a ! -s "$STUB_CARGO_LOG"
newhome
mkdir -p "$tmp/nocargo"
HOME="$home" PATH="$tmp/nocargo:/usr/bin:/bin" ./install.py --agent grok >"$tmp/out" 2>"$tmp/err"; code=$?
is "missing cargo refuses" test "$code" -eq 1 -a ! -e "$home/.agents"
has "missing cargo names the way out" "cargo not found; install Rust or rerun with --skip-binary" "$tmp/err"
newhome
HOME="$home" PATH="$tmp/nocargo:/usr/bin:/bin" ./install.py --agent grok --skip-binary >"$tmp/out" 2>"$tmp/err"; code=$?
is "missing grok CLI is a note, not a failure" test "$code" -eq 0
has "missing grok CLI is named" "note: grok not found on PATH" "$tmp/err"

# --- a failing global MCP command fails the install and says why ---------------
newhome
STUB_MCP_ADD_FAIL="grok add exploded" inst --agent grok --skip-binary; code=$?
is "failing mcp add exits 1" test "$code" -eq 1
has "failing mcp add shows the CLI output" "grok add exploded" "$tmp/err"
newhome
: >"$tmp/mcp.log"
STUB_MCP_LOG="$tmp/mcp.log" STUB_MCP_REMOVE_FAIL="remove denied" inst --agent claude --skip-binary; code=$?
is "unexpected claude remove failure exits 1" test "$code" -eq 1
has "claude remove failure shows the CLI output" "remove denied" "$tmp/err"
is "claude add never runs after a failed remove" bash -c '! grep -q add-json "$1"' _ "$tmp/mcp.log"
newhome
STUB_MCP_REMOVE_FAIL='No MCP server named "agentboard" in user scope' inst --agent claude --skip-binary; code=$?
is "claude remove of an absent entry is tolerated" test "$code" -eq 0
newhome
STUB_MCP_REMOVE_FAIL='Error: MCP server "agentboard" not found' inst --agent agy --skip-binary; code=$?
is "agy remove of an absent entry is tolerated" test "$code" -eq 0
newhome
STUB_MCP_REMOVE_FAIL="remove denied" inst --agent agy --skip-binary; code=$?
is "unexpected agy remove failure exits 1" test "$code" -eq 1

# --- one global agentboard MCP entry per client, through the real CLIs -----------
if [ -n "$real_codex" ] && [ -n "$real_codewhale" ] && [ -n "$real_claude" ] && [ -n "$real_grok" ]; then
  export real_codex real_codewhale real_claude real_grok
  newhome
  seed() { HOME="$home" PATH="$tmp/bin:$PATH" "$@" >/dev/null 2>&1; }
  seed "$real_claude" mcp add-json -s user keep '{"type":"http","url":"http://127.0.0.1:9/keep"}'
  seed "$real_codex" mcp add keep --url http://127.0.0.1:9/keep
  seed "$real_grok" mcp add -s user --transport http keep http://127.0.0.1:9/keep
  seed "$real_codewhale" mcp add keep --url http://127.0.0.1:9/keep
  global_shape() { # one agentboard entry per client, shaped as stated, keep survives
    HOME="$home" python3 - <<'PY'
import json, os, tomllib
home, url = os.environ["HOME"], "http://127.0.0.1:8765/mcp"
bearer = "Bearer ${AGENTBOARD_ACTOR}"
def cfg(path, toml):
    with open(path, "rb") as f:
        return tomllib.load(f) if toml else json.load(f)
claude = cfg(f"{home}/.claude.json", False)["mcpServers"]
codex = cfg(f"{home}/.codex/config.toml", True)["mcp_servers"]
grok = cfg(f"{home}/.grok/config.toml", True)["mcp_servers"]
whale = cfg(f"{home}/.codewhale/mcp.json", False)["servers"]
for name, servers in (("claude", claude), ("codex", codex), ("grok", grok), ("codewhale", whale)):
    assert [k for k in servers if k == "agentboard"] == ["agentboard"], name
    assert "keep" in servers, name
    assert servers["agentboard"]["url"] == url, name
assert claude["agentboard"]["headers"]["Authorization"] == bearer
assert grok["agentboard"]["headers"]["Authorization"] == bearer
assert codex["agentboard"]["bearer_token_env_var"] == "AGENTBOARD_ACTOR"
assert whale["agentboard"]["bearer_token_env_var"] == "AGENTBOARD_ACTOR"
PY
  }
  for run in first second; do
    inst --skip-binary; code=$?
    is "global MCP $run install exits 0" test "$code" -eq 0
    is "global MCP $run install: one agentboard entry per client, keep survives" global_shape
  done
  unset real_codex real_codewhale real_claude real_grok
else
  echo "SKIP: global MCP cases need claude, codex, grok, codewhale"
fi

# --- agy: one stdio-shim entry, replacing any earlier one, through the real CLI ---
if [ -n "$real_agy" ]; then
  export real_agy
  newhome
  shim=$repo/seats/agy/agentboard-stdio
  seed() { HOME="$home" PATH="$tmp/bin:$PATH" "$@" >/dev/null 2>&1; }
  seed "$real_agy" mcp add keep /bin/true
  seed "$real_agy" mcp add -t http agentboard http://127.0.0.1:8765/mcp/antigravity
  agy_shape() {
    python3 - "$home/.gemini/config/mcp_config.json" "$shim" <<'PY'
import json, sys
servers = json.load(open(sys.argv[1]))["mcpServers"]
assert [k for k in servers if k == "agentboard"] == ["agentboard"]
entry = servers["agentboard"]
assert entry["command"] == sys.argv[2] and "serverUrl" not in entry and "args" not in entry and "env" not in entry, entry
assert "keep" in servers
PY
  }
  inst --agent agy --skip-binary; code=$?
  is "agy first install exits 0" test "$code" -eq 0
  is "agy first install: one shim entry replaces the serverUrl entry, keep survives" agy_shape
  cp "$home/.gemini/config/mcp_config.json" "$tmp/agy-config-first"
  inst --agent agy --skip-binary; code=$?
  is "agy second install exits 0" test "$code" -eq 0
  is "agy second install: still one shim entry" agy_shape
  is "agy second install leaves the config byte-identical" cmp -s "$tmp/agy-config-first" "$home/.gemini/config/mcp_config.json"
  is "agy install never writes the CLI settings" test ! -e "$home/.gemini/antigravity-cli/settings.json"
  is "agy skill resolves to the repo skill" test "$(readlink -f "$home/.gemini/config/skills/agentboard-analyst")" = "$repo/skills/agentboard-analyst"
  unset real_agy
else
  echo "SKIP: agy cases need agy"
fi

# --- help and the three-flag surface -----------------------------------------
newhome
for args in -h --help; do
  inst $args; code=$?
  is "$args exits 0" test "$code" -eq 0
  has "$args shows usage" 'usage: install.py [-h] [--agent AGENT[,AGENT...]] [--force] [--skip-binary]' "$tmp/out"
  has "$args lists client names" 'codex,codewhale,claude,grok,agy' "$tmp/out"
  is "$args writes nothing" test ! -e "$home/.agents" -a ! -e "$home/.local/bin"
done
for flag in --grok-only --skip-grok '--codewhale-provider custom' --bogus; do
  # shellcheck disable=SC2086
  inst $flag; code=$?
  is "removed flag $flag exits usage 2" test "$code" -eq 2 -a ! -e "$home/.agents"
  has "removed flag $flag prints usage" 'usage: install.py [-h] [--agent AGENT[,AGENT...]] [--force] [--skip-binary]' "$tmp/err"
done

echo "$((n - fail))/$n passed (installer)"
[ "$fail" -eq 0 ]
