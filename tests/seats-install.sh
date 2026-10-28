#!/usr/bin/env bash
# seats/install.py against isolated homes: foreign links, and the managed block
# it keeps inside the user's rules/default.rules. Nothing touches a real home.
set -u
cd "$(dirname "$0")/.."
repo=$(pwd)
src=$repo/seats/codex/rules/agentboard.rules
launcher=$repo/seats/start-agentboard-manager
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

# --- foreign claude links are refused, then --force replaces them ---
newhome
mkdir -p "$home/.codex/rules"
for seat in analyst coder manager reviewer; do
  ln -s "/old/checkout/seats/claude/$seat-mcp.json" "$home/.claude/$seat-mcp.json"
done
inst; code=$?
is "foreign links refused" test "$code" -eq 1
has "foreign link text" 'use --force to replace it' "$tmp/err"
is "foreign refusal prints nothing on stdout" test ! -s "$tmp/out"
is "foreign refusal writes no rules" test ! -e "$rules"
inst --force
for seat in analyst coder manager reviewer; do
  is "force relinks $seat" test "$(readlink "$home/.claude/$seat-mcp.json")" = "$repo/seats/claude/$seat-mcp.json"
done

# --- missing rules dir: the block alone, no second rules file ---------------
newhome
inst; code=$?
is "fresh install exit" test "$code" -eq 0
is "manager launcher linked" test "$(readlink "$home/.local/bin/start-agentboard-manager")" = "$launcher"
same "fresh default.rules is the source block" "$src" "$rules"
is "no separate agentboard.rules" test ! -e "$home/.codex/rules/agentboard.rules"
has "fresh install says added" "added: $rules" "$tmp/out"
has "done line kept" "done: the codex base config is merged by hand" "$tmp/out"

# --- manager launcher link follows the same foreign-link rules ---------------
newhome
mkdir -p "$home/.local/bin"
ln -s /old/checkout/start-agentboard-manager "$home/.local/bin/start-agentboard-manager"
inst; code=$?
is "foreign launcher link refused" test "$code" -eq 1
has "foreign launcher link diagnosed" 'foreign link:' "$tmp/err"
is "foreign launcher link preserved" test "$(readlink "$home/.local/bin/start-agentboard-manager")" = /old/checkout/start-agentboard-manager
inst --force
is "force replaces launcher link" test "$(readlink "$home/.local/bin/start-agentboard-manager")" = "$launcher"

# --- grok-only does not manage the PATH launcher ------------------------------
newhome
grok_inst
is "grok-only leaves manager launcher alone" test ! -e "$home/.local/bin/start-agentboard-manager"

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
has "Grok force replaces the literal actor" 'AGENTBOARD_ACTOR = "${AGENTBOARD_SEAT:-manager}"' "$grok"
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
has "Grok force restores source actor" 'AGENTBOARD_ACTOR = "${AGENTBOARD_SEAT:-manager}"' "$grok"

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
  is "Grok inside-block refusal links no Claude card" test ! -e "$home/.claude/coder-mcp.json"
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
  is "Grok inline refusal links no Claude card" test ! -e "$home/.claude/coder-mcp.json"
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
is "Grok preflight refusal links no Claude card" test ! -e "$home/.claude/coder-mcp.json"

echo "$((n - fail))/$n passed (seats installer)"
[ "$fail" -eq 0 ]
