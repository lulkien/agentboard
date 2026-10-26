#!/usr/bin/env bash
# seats/install.sh against isolated homes: foreign links, and the managed block
# it keeps inside the user's rules/default.rules. Nothing touches a real home.
set -u
cd "$(dirname "$0")/.."
repo=$(pwd)
src=$repo/seats/codex/rules/agentboard.rules
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
  mkdir -p "$home/.claude" "$home/.config/opencode"
}
inst() { # inst [args]: run the installer; stdout in $tmp/out, stderr in $tmp/err
  HOME="$home" CODEX_HOME="$home/.codex" bash ./seats/install.sh "$@" >"$tmp/out" 2>"$tmp/err"
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

# --- foreign claude/opencode links are refused, then --force replaces them ---
newhome
mkdir -p "$home/.codex/rules"
for seat in analyst coder manager reviewer; do
  ln -s "/old/checkout/seats/claude/$seat-mcp.json" "$home/.claude/$seat-mcp.json"
done
ln -s /old/checkout/seats/opencode/agentboard-mcp.sh "$home/.config/opencode/agentboard-mcp.sh"
inst; code=$?
is "foreign links refused" test "$code" -eq 1
has "foreign link text" 'use --force to replace it' "$tmp/err"
is "foreign refusal prints nothing on stdout" test ! -s "$tmp/out"
is "foreign refusal writes no rules" test ! -e "$rules"
inst --force
for seat in analyst coder manager reviewer; do
  is "force relinks $seat" test "$(readlink "$home/.claude/$seat-mcp.json")" = "$repo/seats/claude/$seat-mcp.json"
done
is "force relinks opencode" test "$(readlink "$home/.config/opencode/agentboard-mcp.sh")" = "$repo/seats/opencode/agentboard-mcp.sh"

# --- missing rules dir: the block alone, no second rules file ---------------
newhome
inst; code=$?
is "fresh install exit" test "$code" -eq 0
same "fresh default.rules is the source block" "$src" "$rules"
is "no separate agentboard.rules" test ! -e "$home/.codex/rules/agentboard.rules"
has "fresh install says added" "added: $rules" "$tmp/out"
has "done line kept" "done: the codex base config is merged by hand" "$tmp/out"

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

echo "$((n - fail))/$n passed (seats installer)"
[ "$fail" -eq 0 ]
