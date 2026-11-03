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
printf 'CALL' >>"$STUB_HC_LOG"
printf ' <%s>' "$@" >>"$STUB_HC_LOG"
printf '\n' >>"$STUB_HC_LOG"
case $1 in
  run) echo '{"address":"t9"}' ;;
  send) echo '{"started":true}' ;;
  wait) echo '{"wait":"exit"}' ;;
esac
SH
cat >"$tmp/bin/herdr" <<'SH'
#!/usr/bin/env bash
printf 'CALL' >>"$HERDR_LOG"
printf ' <%s>' "$@" >>"$HERDR_LOG"
printf '\n' >>"$HERDR_LOG"
if [[ $1 == pane && $2 == process-info ]]; then
  case ${HERDR_MODE:-ready} in
    ready) echo '{"result":{"process_info":{"foreground_process_group_id":4,"shell_pid":4}}}' ;;
    missing) echo '{"result":{}}' ;;
    mismatch) echo '{"result":{"process_info":{"foreground_process_group_id":5,"shell_pid":4}}}' ;;
  esac
fi
SH
cat >"$tmp/bin/sleep" <<'SH'
#!/usr/bin/env bash
exit 0
SH
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH" HC_TERMINAL_ID=t4 STUB_HC_LOG="$tmp/hc.log" HERDR_LOG="$tmp/herdr.log" XDG_RUNTIME_DIR="$tmp"
# hc cases run outside herdr; the herdr cases set the pane and tab ids themselves
unset HERDR_PANE_ID HERDR_TAB_ID HERDR_ENV
no_hc() { local v; for v in $(compgen -e HC_); do unset "$v"; done; }
export HOME="$tmp/home"
repo=$(pwd)

run_kind() {
  local kind=$1 expected=$2 wait_at send_at
  : >"$STUB_HC_LOG"
  if "$repo/seats/start-agentboard-manager" "$kind" >"$tmp/out" 2>"$tmp/err"; then ok; else bad "$kind launcher succeeds"; fi
  exact "$STUB_HC_LOG" "$expected" "$kind command starts manager"
  exact "$tmp/out" 'manager t9' "$kind prints the address"
  if [[ $kind == grok ]]; then
    exact "$STUB_HC_LOG" 'CALL <wait> <manager>' "$kind waits for the terminal"
    exact "$STUB_HC_LOG" 'CALL <send> <--wait> <manager> </agentboard-manager>' "$kind sends the entry prompt"
    wait_at=$(grep -nFx -- 'CALL <wait> <manager>' "$STUB_HC_LOG" | cut -d: -f1 | head -1 || true)
    send_at=$(grep -nFx -- 'CALL <send> <--wait> <manager> </agentboard-manager>' "$STUB_HC_LOG" | cut -d: -f1 | head -1 || true)
    is "$kind waits before sending" bash -c '[[ -n $1 && -n $2 && $1 -lt $2 ]]' _ "$wait_at" "$send_at"
  else
    is "$kind sends nothing" bash -c '! grep -Fq "CALL <send>" "$1"' _ "$STUB_HC_LOG"
    is "$kind waits for nothing" bash -c '! grep -Fq "CALL <wait>" "$1"' _ "$STUB_HC_LOG"
  fi
}
run_kind claude 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <claude> <--model> <claude-sonnet-5-5> <--mcp-config> <'$HOME'/.claude/manager-mcp.json> <--dangerously-skip-permissions> </agentboard-manager>'
run_kind codex 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <codex> <--no-daemon> <--profile> <manager> <--model> <gpt-6-luna> <$agentboard-manager>'
run_kind grok 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <grok> <--trust> <--always-approve>'

run_neither() {
  : >"$STUB_HC_LOG"
  : >"$HERDR_LOG"
  if (no_hc; export HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>"$tmp/neither"; then
    bad 'neither environment unexpectedly succeeded'
  else
    is 'neither environment exits 1' test "$?" -eq 1
  fi
  has "$tmp/neither" 'inside herdr (HERDR_ENV=1) or hc (any HC_* variable set)' 'neither environment says why'
  is 'neither environment starts nothing in hc' bash -c '! grep -Fq "CALL <run>" "$1"' _ "$STUB_HC_LOG"
  is 'neither environment starts nothing in herdr' test ! -s "$HERDR_LOG"
}
run_neither

# any HC_* variable selects hc, not only HC_TERMINAL_ID
: >"$STUB_HC_LOG"
if (no_hc; export HC_OTHER=1; "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>&1; then ok; else bad 'HC_OTHER alone selects hc'; fi
has "$STUB_HC_LOG" 'CALL <run>' 'HC_OTHER alone starts the manager in hc'

# herdr branch: HERDR_ENV=1 without the pane and tab ids is refused
run_half_herdr() {
  : >"$HERDR_LOG"
  if (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7; "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>"$tmp/half"; then
    bad 'half a herdr environment unexpectedly succeeded'
  else
    is 'half a herdr environment exits 1' test "$?" -eq 1
  fi
  is 'half a herdr environment starts nothing' test ! -s "$HERDR_LOG"
}
run_half_herdr

run_herdr_kind() {
  local kind=$1 expected=$2
  : >"$HERDR_LOG"; : >"$STUB_HC_LOG"
  (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/start-agentboard-manager" "$kind") >"$tmp/out" 2>"$tmp/err"
  for _ in {1..50}; do
    grep -qF 'CALL <agent> <start> <manager>' "$HERDR_LOG" && break
    /usr/bin/sleep 0.02
  done
  exact "$HERDR_LOG" "$expected" "herdr $kind command starts manager"
  exact "$HERDR_LOG" 'CALL <tab> <rename> <tab-3> <manager>' "herdr $kind renames tab"
  if [[ $kind == grok ]]; then
    for _ in {1..50}; do
      grep -qF 'CALL <agent> <prompt>' "$HERDR_LOG" && break
      /usr/bin/sleep 0.02
    done
    exact "$HERDR_LOG" 'CALL <agent> <prompt> <manager> </agentboard-manager> <--wait>' "herdr $kind sends the first prompt"
  else
    is "herdr $kind sends no prompt" bash -c '! grep -Fq "CALL <agent> <prompt>" "$1"' _ "$HERDR_LOG"
  fi
  is "herdr $kind never calls hc" test ! -s "$STUB_HC_LOG"
}
run_herdr_kind claude 'CALL <agent> <start> <manager> <--kind> <claude> <--pane> <pane-7> <--> <--model> <claude-sonnet-5-5> <--mcp-config> <'$HOME'/.claude/manager-mcp.json> <--dangerously-skip-permissions> <--> </agentboard-manager>'
run_herdr_kind codex 'CALL <agent> <start> <manager> <--kind> <codex> <--pane> <pane-7> <--> <--profile> <manager> <--model> <gpt-6-luna> <--> <$agentboard-manager>'
run_herdr_kind grok 'CALL <agent> <start> <manager> <--kind> <grok> <--pane> <pane-7> <--> <--trust> <--always-approve>'

run_herdr_not_ready() {
  local mode=$1
  : >"$HERDR_LOG"
  : >"$XDG_RUNTIME_DIR/agentboard-manager-start.log"
  (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3 HERDR_MODE=$mode; "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>&1
  for _ in {1..100}; do
    grep -qF 'timed out waiting for shell prompt' "$XDG_RUNTIME_DIR/agentboard-manager-start.log" && break
    /usr/bin/sleep 0.02
  done
  is "herdr $mode info does not start agent" bash -c '! grep -Fq "CALL <agent> <start>" "$1"' _ "$HERDR_LOG"
  has "$XDG_RUNTIME_DIR/agentboard-manager-start.log" 'timed out waiting for shell prompt' "herdr $mode info times out"
}
run_herdr_not_ready missing
run_herdr_not_ready mismatch

# both environments present: herdr wins, hc is not called
: >"$HERDR_LOG"; : >"$STUB_HC_LOG"
(export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>&1
has "$HERDR_LOG" 'CALL <tab> <rename> <tab-3> <manager>' 'both environments pick herdr'
is 'both environments leave hc alone' test ! -s "$STUB_HC_LOG"

run_missing_herdr() {
  if (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; PATH="$tmp/nohc" "$repo/seats/start-agentboard-manager" claude) >"$tmp/out" 2>"$tmp/no-herdr"; then
    bad 'missing herdr unexpectedly succeeded'
  else
    is 'missing herdr exits 1' test "$?" -eq 1
  fi
  has "$tmp/no-herdr" 'required command not found: herdr' 'missing herdr is reported'
}

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
run_missing_herdr

if "$repo/seats/start-agentboard-manager" >"$tmp/out" 2>"$tmp/no-kind"; then bad 'no kind unexpectedly succeeded'; else
  is 'no kind exits 1' test "$?" -eq 1
fi
has "$tmp/no-kind" 'usage: start-agentboard-manager {claude|codex|grok}' 'no kind reports usage'
is 'unknown kind exits 1' bash -c '"$1" invalid >/dev/null 2>&1; [[ $? -eq 1 ]]' _ "$repo/seats/start-agentboard-manager"
if "$repo/seats/start-agentboard-manager" invalid >"$tmp/out" 2>"$tmp/usage"; then bad 'unknown kind unexpectedly succeeded'; else ok; fi
has "$tmp/usage" 'usage: start-agentboard-manager {claude|codex|grok}' 'unknown kind reports usage'

echo "$((n - fail))/$n passed (manager launcher)"
[[ $fail -eq 0 ]]
