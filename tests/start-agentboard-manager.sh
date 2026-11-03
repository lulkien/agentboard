#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
n=0 fail=0
ok() { n=$((n + 1)); }
bad() { n=$((n + 1)); fail=$((fail + 1)); echo "FAIL: $*" >&2; }
is() { local name=$1; shift; if "$@"; then ok; else bad "$name"; fi; }
has() { if grep -Fq -- "$2" "$1"; then ok; else bad "$3"; fi; }
exact() { if grep -Fxq -- "$2" "$1"; then ok; else bad "$3"; fi; }

mkdir "$tmp/bin"
cat >"$tmp/bin/hc" <<'SH'
#!/usr/bin/env bash
printf 'CALL' >>"$HC_LOG"
printf ' <%s>' "$@" >>"$HC_LOG"
printf '\n' >>"$HC_LOG"
case $1 in
  run) echo '{"address":"t9"}' ;;
  send) echo '{"started":true}' ;;
  wait) echo '{"wait":"exit"}' ;;
esac
SH
chmod +x "$tmp/bin/hc"
export PATH="$tmp/bin:$PATH" HC_TERMINAL_ID=t4 HC_LOG="$tmp/hc.log" XDG_RUNTIME_DIR="$tmp"
export HOME="$tmp/home"
repo=$(pwd)

run_kind() {
  local kind=$1 expected=$2 wait_at send_at
  : >"$HC_LOG"
  if "$repo/seats/start-agentboard-manager" "$kind" >"$tmp/out" 2>"$tmp/err"; then ok; else bad "$kind launcher succeeds"; fi
  exact "$HC_LOG" "$expected" "$kind command starts manager"
  exact "$tmp/out" 'manager t9' "$kind prints the address"
  if [[ $kind == grok ]]; then
    exact "$HC_LOG" 'CALL <wait> <manager>' "$kind waits for the terminal"
    exact "$HC_LOG" 'CALL <send> <--if> <idle> <--wait> <manager> </agentboard-manager>' "$kind sends the entry prompt"
    wait_at=$(grep -nFx -- 'CALL <wait> <manager>' "$HC_LOG" | cut -d: -f1 | head -1 || true)
    send_at=$(grep -nFx -- 'CALL <send> <--if> <idle> <--wait> <manager> </agentboard-manager>' "$HC_LOG" | cut -d: -f1 | head -1 || true)
    is "$kind waits before sending" bash -c '[[ -n $1 && -n $2 && $1 -lt $2 ]]' _ "$wait_at" "$send_at"
  else
    is "$kind sends nothing" bash -c '! grep -Fq "CALL <send>" "$1"' _ "$HC_LOG"
    is "$kind waits for nothing" bash -c '! grep -Fq "CALL <wait>" "$1"' _ "$HC_LOG"
  fi
}
run_kind claude 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <claude> <--model> <claude-sonnet-5-5> <--mcp-config> <'$HOME'/.claude/manager-mcp.json> <--dangerously-skip-permissions> </agentboard-manager>'
run_kind codex 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <codex> <--no-daemon> <--profile> <manager> <--model> <gpt-6-luna> <$agentboard-manager>'
run_kind grok 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <grok> <--trust> <--always-approve>'

run_no_terminal() {
  : >"$HC_LOG"
  if (unset HC_TERMINAL_ID; "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>"$tmp/no-terminal"; then
    bad 'unset HC_TERMINAL_ID unexpectedly succeeded'
  else
    is 'unset HC_TERMINAL_ID exits 1' test "$?" -eq 1
  fi
  has "$tmp/no-terminal" 'HC_TERMINAL_ID is required' 'unset HC_TERMINAL_ID says why'
  is 'unset HC_TERMINAL_ID starts nothing' bash -c '! grep -Fq "CALL <run>" "$1"' _ "$HC_LOG"
}
run_no_terminal

run_missing_hc() {
  mkdir "$tmp/nohc"
  ln -s "$(command -v bash)" "$tmp/nohc/bash"
  ln -s "$(command -v jq)" "$tmp/nohc/jq"
  if PATH="$tmp/nohc" "$repo/seats/start-agentboard-manager" claude >"$tmp/out" 2>"$tmp/no-hc"; then
    bad 'missing hc unexpectedly succeeded'
  else
    is 'missing hc exits 1' test "$?" -eq 1
  fi
  has "$tmp/no-hc" 'required command not found: hc' 'missing hc is reported'
}
run_missing_hc

if "$repo/seats/start-agentboard-manager" >"$tmp/out" 2>"$tmp/no-kind"; then bad 'no kind unexpectedly succeeded'; else
  is 'no kind exits 1' test "$?" -eq 1
fi
has "$tmp/no-kind" 'usage: start-agentboard-manager {claude|codex|grok}' 'no kind reports usage'
is 'unknown kind exits 1' bash -c '"$1" invalid >/dev/null 2>&1; [[ $? -eq 1 ]]' _ "$repo/seats/start-agentboard-manager"
if "$repo/seats/start-agentboard-manager" invalid >"$tmp/out" 2>"$tmp/usage"; then bad 'unknown kind unexpectedly succeeded'; else ok; fi
has "$tmp/usage" 'usage: start-agentboard-manager {claude|codex|grok}' 'unknown kind reports usage'

echo "$((n - fail))/$n passed (manager launcher)"
[[ $fail -eq 0 ]]
