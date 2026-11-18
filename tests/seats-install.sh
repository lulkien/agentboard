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
  mkdir -p "$home/.claude"
}
inst() { # inst [args]: run the installer; stdout in $tmp/out, stderr in $tmp/err
  HOME="$home" CODEX_HOME="$home/.codex" python3 ./seats/install.py "$@" >"$tmp/out" 2>"$tmp/err"
}
grok_inst() { # grok_inst [args]: Grok-only install into the isolated home
  HOME="$home" python3 ./seats/install.py --grok-only "$@" >"$tmp/out" 2>"$tmp/err"
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

# --- generated Claude seat files: merge, idempotence and safe replacement ---
newhome
mkdir -p "$home/.codex/rules"
printf '%s' '{"mcpServers":{"agentboard":{"type":"http","url":"http://wrong"},"extra":{"type":"stdio","command":"x","env":{"TOKEN":"s3cret-value"}}}}' >"$home/.claude.json"
cp -r "$repo/seats/claude" "$tmp/cards-before"
inst; code=$?
is "generated install exit" test "$code" -eq 0
for seat in analyst coder manager reviewer; do
  is "$seat generated regular 0600 merged file" python3 -c 'import json,os,stat,sys; p,s=sys.argv[1:]; d=json.load(open(p)); card=json.load(open("seats/claude/"+s+"-mcp.json")); assert stat.S_IMODE(os.stat(p).st_mode)==0o600 and not os.path.islink(p); assert d["mcpServers"]["agentboard"]==card["mcpServers"]["agentboard"] and "extra" in d["mcpServers"]' "$home/.claude/$seat-mcp.json" "$seat"
done
has "names only output" "servers: agentboard, codebase-memory-mcp, extra" "$tmp/out"
is "generated lines list names in sorted order" bash -c 'grep -q "servers: agentboard, codebase-memory-mcp, extra" "$1"' _ "$tmp/out"
is "secret not printed" bash -c '! grep -q s3cret-value "$1" "$2"' _ "$tmp/out" "$tmp/err"
same "user config unchanged" "$home/.claude.json" "$home/.claude.json"
diff -r "$tmp/cards-before" "$repo/seats/claude" >/dev/null || bad "repo cards unchanged"
inst
has "rerun unchanged" "unchanged: $home/.claude/coder-mcp.json" "$tmp/out"
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
is "user config remains unchanged" grep -q 'wrong' "$home/.claude.json"
is "user config was not decorated with installer marker" bash -c '! grep -q agentboardGenerated "$1"' _ "$home/.claude.json"
inst
is "second run all unchanged" bash -c 'grep -c "^unchanged: .*/.claude/.*mcp.json$" "$1" | grep -qx 4' _ "$tmp/out"
is "unchanged file bytes stable" cmp -s "$home/.claude/coder-mcp.json" "$home/.claude/coder-mcp.json"
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
inst --force; code=$?
is "force refuses unmarked regular file" test "$code" -eq 1
is "unmarked regular file preserved" test "$(cat "$home/.claude/manager-mcp.json")" = mine
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

# --- managed Grok block: source bytes, idempotence and force behavior --------
newhome
grok=$home/.grok/config.toml
grok_inst; code=$?
is "fresh Grok install exit" test "$code" -eq 0
same "fresh Grok config is the source block" "$grok_src" "$grok"
has "fresh Grok config has seat HTTP URL" 'url = "http://127.0.0.1:8765/mcp/${AGENTBOARD_SEAT:-manager}"' "$grok"
is "fresh Grok config has no stdio serve" test -z "$(grep -F 'args = ["serve"]' "$grok" || true)"
has "fresh Grok install says added" "added: $grok" "$tmp/out"
is "Grok-only install leaves Codex links alone" test ! -e "$home/.codex/coder.config.toml"
cp "$grok" "$tmp/grok-before"
grok_inst
same "Grok rerun is byte-identical" "$tmp/grok-before" "$grok"
has "Grok rerun says unchanged" 'unchanged:' "$tmp/out"
is "Grok rerun has no differs note" test ! -s "$tmp/err"
grok_inst --force
same "Grok force on current source is byte-identical" "$tmp/grok-before" "$grok"

newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
sed 's/AGENTBOARD_SEAT:-manager/manager/' "$grok_src" >"$grok"
cp "$grok" "$tmp/grok-literal-actor"
grok_inst
same "Grok plain run leaves a different literal actor" "$tmp/grok-literal-actor" "$grok"
has "Grok different literal actor reports difference" 'differs from the repo' "$tmp/err"
has "Grok different literal actor says unchanged" 'unchanged:' "$tmp/out"
grok_inst --force
has "Grok force replaces the literal actor" 'url = "http://127.0.0.1:8765/mcp/${AGENTBOARD_SEAT:-manager}"' "$grok"
has "Grok force on literal actor says replaced" 'replaced:' "$tmp/out"

# --- preserve existing config bytes, other tables, CRLF and file mode -------
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
printf '[ui]\ntheme = "dark"\n\n[mcp_servers.other]\ncommand = "other"' >"$grok"
cp "$grok" "$tmp/grok-prefix"
grok_inst
is "Grok append preserves no-final-LF prefix" cmp -s -n "$(wc -c <"$tmp/grok-prefix")" "$tmp/grok-prefix" "$grok"
is "Grok leaves other MCP table" grep -qF '[mcp_servers.other]' "$grok"
sum=$(cksum <"$grok")
grok_inst
is "Grok config rerun remains stable with other tables" test "$(cksum <"$grok")" = "$sum"

newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
printf '[ui]\r\ntheme = "dark"\r\n' >"$grok"
chmod 640 "$grok"
grok_inst
is "Grok CRLF prefix preserved" grep -q $'theme = "dark"\r$' "$grok"
is "Grok appended block uses CRLF" test "$(grep -c $'\r$' "$grok")" -eq "$(wc -l <"$grok")"
is "Grok config mode preserved" test "$(stat -c %a "$grok")" = 640
sum=$(cksum <"$grok")
grok_inst --force
is "Grok CRLF force is byte-stable" test "$(cksum <"$grok")" = "$sum"
is "Grok temp file removed" test -z "$(ls "$home/.grok" | grep -F .agentboard || true)"

# --- marked block force behavior and malformed marker refusals --------------
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
{
  echo '[ui]'
  echo 'theme = "light"'
  sed 's/AGENTBOARD_SEAT:-manager/edited/' "$grok_src"
  echo '[mcp_servers.other]'
  echo 'command = "other"'
} >"$grok"
cp "$grok" "$tmp/grok-edited"
grok_inst
same "Grok plain run leaves edited block" "$tmp/grok-edited" "$grok"
has "Grok plain run notes edited block" 'differs from the repo' "$tmp/err"
grok_inst --force
has "Grok force replaces marked block" 'replaced:' "$tmp/out"
is "Grok force keeps neighboring tables" grep -qF '[mcp_servers.other]' "$grok"
has "Grok force restores source actor" 'url = "http://127.0.0.1:8765/mcp/${AGENTBOARD_SEAT:-manager}"' "$grok"

# An existing marked stdio card upgrades in place under --force.
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
cat >"$grok" <<'EOF'
# BEGIN AGENTBOARD MCP
[mcp_servers.agentboard]
command = "agentboard"
args = ["serve"]
enabled = true

[mcp_servers.agentboard.env]
AGENTBOARD_ACTOR = "${AGENTBOARD_SEAT:-manager}"
# END AGENTBOARD MCP
EOF
grok_inst --force
has "Grok force upgrades stdio card to HTTP" 'url = "http://127.0.0.1:8765/mcp/${AGENTBOARD_SEAT:-manager}"' "$grok"
is "Grok upgraded card has no serve" test -z "$(grep -F 'args = ["serve"]' "$grok" || true)"

GB='# BEGIN AGENTBOARD MCP'
GE='# END AGENTBOARD MCP'
for kind in duplicate_begin duplicate_end begin_only end_before_begin padded; do
  case $kind in
    duplicate_begin) printf '%s\nx\n%s\n%s\n' "$GB" "$GB" "$GE" >"$tmp/body" ;;
    duplicate_end) printf '%s\n%s\n%s\n' "$GB" "$GE" "$GE" >"$tmp/body" ;;
    begin_only) printf '[ui]\n%s\nx\n' "$GB" >"$tmp/body" ;;
    end_before_begin) printf '%s\n%s\n' "$GE" "$GB" >"$tmp/body" ;;
    padded) printf ' %s\nx\n%s\n' "$GB" "$GE" >"$tmp/body" ;;
  esac
  for args in "" "--force"; do
    newhome
    grok=$home/.grok/config.toml
    mkdir -p "$home/.grok"
    cp "$tmp/body" "$grok"
    cp "$grok" "$tmp/grok-before"
    grok_inst $args; code=$?
    is "Grok malformed $kind ${args:-plain} refused" test "$code" -eq 1
    same "Grok malformed $kind ${args:-plain} unchanged" "$tmp/grok-before" "$grok"
    has "Grok malformed $kind ${args:-plain} diagnosed" 'malformed agentboard block' "$tmp/err"
  done
done

# --- unmarked tables, malformed TOML and unsafe destinations refuse --------
for args in "" "--force"; do
  newhome
  grok=$home/.grok/config.toml
  mkdir -p "$home/.grok"
  printf '[mcp_servers.agentboard]\ncommand = "grok mcp add"\n' >"$grok"
  cp "$grok" "$tmp/grok-before"
  grok_inst $args; code=$?
  is "Grok unmarked table ${args:-plain} refused" test "$code" -eq 1
  same "Grok unmarked table ${args:-plain} unchanged" "$tmp/grok-before" "$grok"
  has "Grok unmarked table ${args:-plain} manual repair" 'remove or convert it by hand' "$tmp/err"
done
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
printf '[ui\n' >"$grok"
grok_inst; code=$?
is "Grok malformed TOML refused" test "$code" -eq 1
has "Grok malformed TOML named" 'invalid TOML' "$tmp/err"

# A valid marker pair does not make invalid TOML inside the block safe to keep.
for args in "" "--force"; do
  newhome
  grok=$home/.grok/config.toml
  mkdir -p "$home/.grok" "$home/.codex/rules" "$home/.claude"
  printf '[ui]\ntheme = "dark"\n# BEGIN AGENTBOARD MCP\n[mcp_servers.agentboard]\nargs = [\n# END AGENTBOARD MCP\n' >"$grok"
  cp "$grok" "$tmp/grok-before"
  inst $args; code=$?
  is "Grok malformed TOML inside block ${args:-plain} refused" test "$code" -eq 1
  same "Grok malformed TOML inside block ${args:-plain} unchanged" "$tmp/grok-before" "$grok"
  has "Grok malformed TOML inside block ${args:-plain} diagnosed" 'invalid TOML' "$tmp/err"
  is "Grok inside-block refusal links no Codex card" test ! -e "$home/.codex/coder.config.toml"
  is "Grok inside-block refusal writes no rules" test ! -e "$home/.codex/rules/default.rules"
done
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok"
printf 'target\n' >"$home/target"
ln -s "$home/target" "$grok"
grok_inst --force; code=$?
is "Grok symlink refused" test "$code" -eq 1
is "Grok symlink target untouched" test "$(cat "$home/target")" = target
newhome
grok=$home/.grok/config.toml
mkdir -p "$grok"
grok_inst --force; code=$?
is "Grok directory refused" test "$code" -eq 1
has "Grok directory refusal named" 'not a regular file' "$tmp/err"

# An inline mcp_servers table is valid alone but cannot accept an appended table.
for args in "" "--force"; do
  newhome
  grok=$home/.grok/config.toml
  mkdir -p "$home/.grok" "$home/.codex/rules" "$home/.claude"
  printf 'mcp_servers = { other = { command = "other" } }\n' >"$grok"
  cp "$grok" "$tmp/grok-before"
  inst $args; code=$?
  is "Grok inline mcp_servers ${args:-plain} refused" test "$code" -eq 1
  same "Grok inline mcp_servers ${args:-plain} unchanged" "$tmp/grok-before" "$grok"
  has "Grok inline mcp_servers ${args:-plain} candidate diagnosed" 'invalid TOML in rendered Grok config' "$tmp/err"
  is "Grok inline refusal links no Codex card" test ! -e "$home/.codex/coder.config.toml"
  is "Grok inline refusal writes no rules" test ! -e "$home/.codex/rules/default.rules"
done

# --- Grok preflight runs before other installer destinations are written ----
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok" "$home/.codex/rules" "$home/.claude"
printf '[mcp_servers.agentboard]\ncommand = "manual"\n' >"$grok"
cp "$grok" "$tmp/grok-before"
inst --force; code=$?
is "full install refuses unmarked Grok table" test "$code" -eq 1
same "full install refusal preserves Grok config" "$tmp/grok-before" "$grok"
is "Grok preflight refusal links no Codex card" test ! -e "$home/.codex/coder.config.toml"
is "Grok preflight refusal writes no rules" test ! -e "$home/.codex/rules/default.rules"

# --- --skip-grok installs everything else without reading or changing Grok --
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok" "$home/.codex/rules"
printf '[mcp_servers.agentboard]\ncommand = "foreign"\n' >"$grok"
cp "$grok" "$tmp/grok-skip-before"
sum=$(cksum <"$grok")
inst --force --skip-grok; code=$?
is "skip-grok full install exits successfully" test "$code" -eq 0
is "skip-grok installs Codex rules block" grep -qF '# BEGIN AGENTBOARD RULES' "$rules"
is "skip-grok generates Claude seat file" test -f "$home/.claude/coder-mcp.json" -a ! -L "$home/.claude/coder-mcp.json"
is "skip-grok installs Codex card" test -L "$home/.codex/coder.config.toml"
same "skip-grok preserves foreign Grok bytes" "$tmp/grok-skip-before" "$grok"
is "skip-grok preserves Grok checksum" test "$(cksum <"$grok")" = "$sum"
is "skip-grok emits no Grok access diagnostic" test ! -s "$tmp/err"

# The default full install keeps its existing refusal for that same entry.
newhome
grok=$home/.grok/config.toml
mkdir -p "$home/.grok" "$home/.codex/rules"
printf '[mcp_servers.agentboard]\ncommand = "foreign"\n' >"$grok"
cp "$grok" "$tmp/grok-default-before"
inst --force; code=$?
is "default full install refuses foreign Grok entry" test "$code" -eq 1
same "default refusal preserves Grok bytes" "$tmp/grok-default-before" "$grok"
has "default refusal keeps unmarked-entry message" 'unmarked [mcp_servers.agentboard] entry' "$tmp/err"
is "default refusal links no Codex card" test ! -e "$home/.codex/coder.config.toml"

# Combining the mutually exclusive Grok modes is a usage error.
newhome
inst --skip-grok --grok-only; code=$?
is "skip-grok with grok-only exits with usage error" test "$code" -eq 2
has "conflicting Grok flags show usage error" 'mutually exclusive' "$tmp/err"

# --- Codewhale per-seat homes: own config + MCP card, never ~/.codewhale ---
newhome
mkdir -p "$home/.codex/rules"
inst; code=$?
is "codewhale install exit" test "$code" -eq 0
cw=$home/.local/share/agentboard/codewhale
for seat in analyst coder manager reviewer; do
  c=$cw/$seat/config.toml
  has "$seat effort" 'reasoning_effort = "medium"' "$c"
  has "$seat mcp path" "mcp_config_path = \"$cw/$seat/mcp.json\"" "$c"
  has "$seat yolo" 'yolo = true' "$c"
  is "$seat mcp card" python3 -c 'import json,sys; assert json.load(open(sys.argv[1]))=={"servers":{"agentboard":{"url":"http://127.0.0.1:8765/mcp/"+sys.argv[2]}}}' "$cw/$seat/mcp.json" "$seat"
  is "$seat has no approval_policy or provider" bash -c '! grep -Eq "approval_policy|^provider" "$1"' _ "$c"
  is "$seat valid TOML" python3 -c 'import sys,tomllib; tomllib.load(open(sys.argv[1],"rb"))' "$c"
done
for seat in manager coder; do
  is "$seat has no sandbox keys" bash -c '! grep -q sandbox "$1"' _ "$cw/$seat/config.toml"
done
has "analyst sandbox" 'sandbox_mode = "workspace-write"' "$cw/analyst/config.toml"
is "analyst no network key" bash -c '! grep -q sandbox_network_access "$1"' _ "$cw/analyst/config.toml"
has "reviewer sandbox" 'sandbox_mode = "workspace-write"' "$cw/reviewer/config.toml"
has "reviewer network" 'sandbox_network_access = true' "$cw/reviewer/config.toml"
is "nothing under ~/.codewhale" test ! -e "$home/.codewhale"
is "no approval_policy in repo templates" bash -c '! grep -rq approval_policy "$1"/seats/codewhale' _ "$repo"
find "$cw" -type f | sort >"$tmp/cw-files"
is "eight generated files" test "$(wc -l <"$tmp/cw-files")" -eq 8
xargs cat <"$tmp/cw-files" >"$tmp/cw-before"
inst
is "codewhale rerun unchanged" bash -c 'grep -c "^unchanged: .*codewhale/.*" "$1" | grep -qx 8' _ "$tmp/out"
xargs cat <"$tmp/cw-files" >"$tmp/cw-after"
same "codewhale rerun byte-identical" "$tmp/cw-before" "$tmp/cw-after"
if command -v codewhale >/dev/null 2>&1; then
  for seat in analyst coder manager reviewer; do
    HOME="$home" CODEWHALE_HOME="$cw/$seat" codewhale mcp list >"$tmp/mcp-list" 2>&1
    is "$seat codewhale mcp list ends /mcp/$seat" bash -c 'grep -q "/mcp/$1\$" "$2"' _ "$seat" "$tmp/mcp-list"
    HOME="$home" CODEWHALE_HOME="$cw/$seat" codewhale config doctor >"$tmp/doctor" 2>&1
    is "$seat codewhale config doctor reports no error" bash -c '! grep -qiE "(^|[^a-z])(error|fail)" "$1"' _ "$tmp/doctor"
  done
  is "probes left ~/.codewhale content-free of config" test ! -e "$home/.codewhale/config.toml"
fi
# a hand-edited config is refused (kept) without --force and replaced with it
cp "$cw/coder/config.toml" "$tmp/cw-coder-orig"
printf 'reasoning_effort = "high"\n' >>"$cw/coder/config.toml"
cp "$cw/coder/config.toml" "$tmp/cw-coder-edited"
inst
has "edited config reported" "unchanged: $cw/coder/config.toml (differs" "$tmp/out"
same "edited config kept without force" "$tmp/cw-coder-edited" "$cw/coder/config.toml"
inst --force
has "edited config replaced with force" "replaced: $cw/coder/config.toml" "$tmp/out"
same "force restores template output" "$tmp/cw-coder-orig" "$cw/coder/config.toml"
# a file we did not generate is never replaced, even with --force
printf 'mine = 1\n' >"$cw/manager/config.toml"
inst --force; code=$?
is "unmarked codewhale config refused with force" test "$code" -eq 1
has "unmarked config message" "not generated by agentboard: $cw/manager/config.toml" "$tmp/err"
has "unmarked config kept" 'mine = 1' "$cw/manager/config.toml"

# --codewhale-provider writes only the provider name; a bad name is a usage error
newhome
mkdir -p "$home/.codex/rules"
inst --codewhale-provider deepseek; code=$?
is "provider install exit" test "$code" -eq 0
for seat in analyst coder manager reviewer; do
  has "$seat provider line" 'provider = "deepseek"' "$home/.local/share/agentboard/codewhale/$seat/config.toml"
  is "$seat provider config valid TOML" python3 -c 'import sys,tomllib; assert tomllib.load(open(sys.argv[1],"rb"))["provider"]=="deepseek"' "$home/.local/share/agentboard/codewhale/$seat/config.toml"
done
is "no credential files written" bash -c '! find "$1" -type f \( -name "*auth*" -o -name "*key*" -o -name "*.env" \) | grep -q .' _ "$home/.local/share/agentboard"
newhome
inst --codewhale-provider 'x"; yolo = false'; code=$?
is "bad provider name usage error" test "$code" -eq 2
is "bad provider writes nothing" test ! -e "$home/.local/share/agentboard"
inst --grok-only; code=$?
is "grok-only writes no codewhale home" test ! -e "$home/.local/share/agentboard"

echo "$((n - fail))/$n passed (seats installer)"
[ "$fail" -eq 0 ]
