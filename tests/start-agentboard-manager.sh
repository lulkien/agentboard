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
  send)
    if [[ -n ${STUB_HC_STARTING:-} && $(cat "$STUB_HC_STARTING") -gt 0 ]]; then
      echo $(($(cat "$STUB_HC_STARTING") - 1)) >"$STUB_HC_STARTING"
      echo 'unavailable: t9 is still starting: its output has not settled' >&2
      exit 1
    fi
    if [[ -n ${STUB_HC_NOSTART:-} ]]; then echo '{"started":false}'; else echo '{"started":true}'; fi ;;
  wait) echo '{"wait":"exit"}' ;;
esac
SH
cat >"$tmp/bin/herdr" <<'SH'
#!/usr/bin/env bash
printf 'CALL' >>"$HERDR_LOG"
printf ' <%s>' "$@" >>"$HERDR_LOG"
printf '\n' >>"$HERDR_LOG"
if [[ $1 == agent && $2 == start && ${3:-} == manager && ${5:-} == codex && ${HERDR_ENV:-} == 1 ]]; then
  printf 'ENV CODEX_HOME=%s\n' "${CODEX_HOME:-}" >>"$HERDR_LOG"
fi
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
mkdir -p "$HOME/.claude"
printf '{}\n' >"$HOME/.claude/manager-mcp.json"
repo=$(pwd)
card="$HOME/.claude/manager-mcp.json"

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
run_kind claude 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <claude> <--model> <claude-sonnet-5-5> <--mcp-config> <'$card'> <--strict-mcp-config> <--dangerously-skip-permissions> </agentboard-manager>'
mv "$card" "$card.saved"
: >"$STUB_HC_LOG"
if "$repo/seats/start-agentboard-manager" claude >"$tmp/out" 2>"$tmp/err"; then bad 'missing card unexpectedly launched'; else ok; fi
has "$tmp/err" 'run seats/install.py' 'missing card names installer'
is 'missing card starts nothing' test ! -s "$STUB_HC_LOG"
mv "$card.saved" "$card"
codex_home="$HOME/.local/share/agentboard/seats/manager/codex"
mkdir -p "$codex_home"
printf 'approval_policy = "never"\n' >"$codex_home/manager.config.toml"
run_kind codex 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <env> <CODEX_HOME='$codex_home'> <codex> <--no-daemon> <--profile> <manager> <$agentboard-manager>'
mv "$codex_home/manager.config.toml" "$codex_home/manager.config.saved"
: >"$STUB_HC_LOG"
if "$repo/seats/start-agentboard-manager" codex >"$tmp/out" 2>"$tmp/err"; then bad 'missing isolated Codex profile unexpectedly launched'; else ok; fi
has "$tmp/err" 'run seats/install.py' 'missing isolated Codex profile names installer'
is 'missing isolated Codex profile starts nothing' test ! -s "$STUB_HC_LOG"
mv "$codex_home/manager.config.saved" "$codex_home/manager.config.toml"
run_kind grok 'CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <grok> <--trust> <--always-approve>'

# codewhale: hc only, per-seat home, status wait, retried send --wait
cwhome="$HOME/.local/share/agentboard/codewhale/manager"
cwexp='CALL <run> <--json> <--bg> <--outlive-parent> <--name> <manager> <--cwd> <'$repo'> <--> <env> <CODEWHALE_HOME='$cwhome'> <codewhale> <--skip-onboarding> <--fresh> <--disable> <subagents>'
cwrun() { : >"$STUB_HC_LOG"; "$repo/seats/start-agentboard-manager" codewhale >"$tmp/out" 2>"$tmp/err"; }
mkdir -p "$cwhome"
printf 'yolo = true\n' >"$cwhome/config.toml"
if cwrun; then ok; else bad 'codewhale launcher succeeds'; fi
exact "$STUB_HC_LOG" "$cwexp" 'codewhale command starts manager'
exact "$tmp/out" 'manager t9' 'codewhale prints the address'
exact "$STUB_HC_LOG" 'CALL <wait> <manager> <--timeout> <30>' 'codewhale waits for reported status'
exact "$STUB_HC_LOG" 'CALL <send> <--wait> <manager> </agentboard-manager>' 'codewhale sends the entry prompt with --wait'
is 'codewhale never waits on output text' bash -c '! grep -Fq "<--output>" "$1"' _ "$STUB_HC_LOG"
wait_at=$(grep -nF 'CALL <wait>' "$STUB_HC_LOG" | cut -d: -f1 | head -1 || true)
send_at=$(grep -nF 'CALL <send>' "$STUB_HC_LOG" | cut -d: -f1 | head -1 || true)
is 'codewhale waits before sending' bash -c '[[ -n $1 && -n $2 && $1 -lt $2 ]]' _ "$wait_at" "$send_at"
# still starting twice, then accepted
echo 2 >"$tmp/starting"
if (export STUB_HC_STARTING="$tmp/starting"; cwrun); then ok; else bad 'codewhale retries a still-starting send'; fi
is 'codewhale sent three times' test "$(grep -Fc 'CALL <send>' "$STUB_HC_LOG")" -eq 3
exact "$tmp/out" 'manager t9' 'codewhale prints the address after retries'
# never settles: bounded, then fails
echo 99 >"$tmp/starting"
if (export STUB_HC_STARTING="$tmp/starting"; cwrun); then bad 'codewhale endless starting unexpectedly succeeded'; else ok; fi
is 'codewhale gives up after ten sends' test "$(grep -Fc 'CALL <send>' "$STUB_HC_LOG")" -eq 10
has "$tmp/err" 'entry prompt did not land' 'codewhale reports an unlanded prompt'
# send accepted but the program did not start working
echo 0 >"$tmp/starting"
if (export STUB_HC_STARTING="$tmp/starting" STUB_HC_NOSTART=1; cwrun); then bad 'codewhale unstarted prompt unexpectedly succeeded'; else ok; fi
has "$tmp/err" 'entry prompt did not land' 'codewhale reports started:false'
# missing home
mv "$cwhome" "$cwhome.saved"
if cwrun; then bad 'missing codewhale home unexpectedly launched'; else ok; fi
has "$tmp/err" 'run seats/install.py' 'missing codewhale home names installer'
is 'missing codewhale home starts nothing' test ! -s "$STUB_HC_LOG"
mv "$cwhome.saved" "$cwhome"
# herdr refuses
: >"$HERDR_LOG"; : >"$STUB_HC_LOG"
if (export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/start-agentboard-manager" codewhale) >"$tmp/out" 2>"$tmp/err"; then bad 'codewhale under herdr unexpectedly succeeded'; else ok; fi
has "$tmp/err" 'herdr' 'herdr refusal names herdr'
has "$tmp/err" 'codewhale' 'herdr refusal names codewhale'
is 'herdr refusal never calls herdr' test ! -s "$HERDR_LOG"
is 'herdr refusal never calls hc' test ! -s "$STUB_HC_LOG"

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
run_herdr_kind claude 'CALL <agent> <start> <manager> <--kind> <claude> <--pane> <pane-7> <--> <--model> <claude-sonnet-5-5> <--mcp-config> <'$card'> <--strict-mcp-config> <--dangerously-skip-permissions> <--> </agentboard-manager>'
run_herdr_kind codex 'CALL <agent> <start> <manager> <--kind> <codex> <--pane> <pane-7> <--> <--profile> <manager> <--> <$agentboard-manager>'
has "$HERDR_LOG" "ENV CODEX_HOME=$codex_home" 'herdr Codex manager receives its isolated home'
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
has "$tmp/no-kind" 'usage: start-agentboard-manager {claude|codex|grok|codewhale}' 'no kind reports usage'
is 'unknown kind exits 1' bash -c '"$1" invalid >/dev/null 2>&1; [[ $? -eq 1 ]]' _ "$repo/seats/start-agentboard-manager"
if "$repo/seats/start-agentboard-manager" invalid >"$tmp/out" 2>"$tmp/usage"; then bad 'unknown kind unexpectedly succeeded'; else ok; fi
has "$tmp/usage" 'usage: start-agentboard-manager {claude|codex|grok|codewhale}' 'unknown kind reports usage'

echo "$((n - fail))/$n passed (manager launcher)"
[[ $fail -eq 0 ]]
