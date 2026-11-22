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
if [[ $1 == agent && $2 == start && ${3:-} == manager && ${HERDR_ENV:-} == 1 ]]; then
  printf 'ENV AGENTBOARD_ACTOR=%s CODEX_HOME=%s CLAUDE_CONFIG_DIR=%s GROK_HOME=%s CODEWHALE_HOME=%s\n' \
    "${AGENTBOARD_ACTOR:-}" "${CODEX_HOME:-}" "${CLAUDE_CONFIG_DIR:-}" "${GROK_HOME:-}" "${CODEWHALE_HOME:-}" >>"$HERDR_LOG"
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
# each client records how it was exec'd: argv, its role-home variable and its pid
for client in claude codex grok codewhale; do
  cat >"$tmp/bin/$client" <<'SH'
#!/usr/bin/env bash
{
  printf 'EXEC %s' "${0##*/}"; printf ' <%s>' "$@"; printf '\n'
  printf 'HOMES AGENTBOARD_ACTOR=%s CODEX_HOME=%s CLAUDE_CONFIG_DIR=%s GROK_HOME=%s CODEWHALE_HOME=%s\n' \
    "${AGENTBOARD_ACTOR:-}" "${CODEX_HOME:-}" "${CLAUDE_CONFIG_DIR:-}" "${GROK_HOME:-}" "${CODEWHALE_HOME:-}"
  printf 'PID %s\n' "$$"
} >>"$CLIENT_LOG"
SH
done
chmod +x "$tmp/bin/"*
export PATH="$tmp/bin:$PATH" HC_TERMINAL_ID=t4 STUB_HC_LOG="$tmp/hc.log" CLIENT_LOG="$tmp/client.log" HERDR_LOG="$tmp/herdr.log" XDG_RUNTIME_DIR="$tmp"
# hc cases run outside herdr; the herdr cases set the pane and tab ids themselves
unset HERDR_PANE_ID HERDR_TAB_ID HERDR_ENV
# inherited role homes must not reach the client: the launcher drops them
export CODEX_HOME=/inherited/codex CLAUDE_CONFIG_DIR=/inherited/claude GROK_HOME=/inherited/grok CODEWHALE_HOME=/inherited/codewhale
no_hc() { local v; for v in $(compgen -e HC_); do unset "$v"; done; }
export HOME="$tmp/home"
repo=$(pwd)

# hc: the launcher execs the client in this terminal (HC_TERMINAL_ID=t4) and never
# runs a new hc terminal; Grok and Codewhale get their entry prompt from a helper.
wait_for() { # wait_for <file> <fixed text>: poll for a background helper's line
  for _ in {1..150}; do grep -qF -- "$2" "$1" 2>/dev/null && return 0; /usr/bin/sleep 0.02; done
  return 1
}
run_kind() {
  local kind=$1 expected=$2
  : >"$STUB_HC_LOG"; : >"$CLIENT_LOG"; : >"$XDG_RUNTIME_DIR/agentboard-manager-start.log"
  if "$repo/seats/agentboard-manager" "$kind" >"$tmp/out" 2>"$tmp/err"; then ok; else bad "$kind launcher succeeds"; fi
  exact "$CLIENT_LOG" "$expected" "$kind runs in this terminal with its entry argv"
  exact "$CLIENT_LOG" 'HOMES AGENTBOARD_ACTOR=manager CODEX_HOME= CLAUDE_CONFIG_DIR= GROK_HOME= CODEWHALE_HOME=' "$kind gets the manager actor and no home variable"
  is "$kind starts no new hc terminal" bash -c '! grep -Fq "CALL <run>" "$1"' _ "$STUB_HC_LOG"
  is "$kind prints nothing" test ! -s "$tmp/out"
  if [[ $kind == grok || $kind == codewhale ]]; then
    wait_for "$STUB_HC_LOG" 'CALL <send> <--wait> <t4> </agentboard-manager>' || true
    exact "$STUB_HC_LOG" 'CALL <wait> <t4> <--timeout> <30>' "$kind helper waits for this terminal"
    exact "$STUB_HC_LOG" 'CALL <send> <--wait> <t4> </agentboard-manager>' "$kind helper types the entry prompt here"
    is "$kind helper waits before sending" bash -c 'w=$(grep -nF "CALL <wait>" "$1" | cut -d: -f1 | head -1); s=$(grep -nF "CALL <send>" "$1" | cut -d: -f1 | head -1); [[ -n $w && -n $s && $w -lt $s ]]' _ "$STUB_HC_LOG"
  else
    /usr/bin/sleep 0.1
    is "$kind sends nothing" bash -c '! grep -Fq "CALL <send>" "$1"' _ "$STUB_HC_LOG"
    is "$kind waits for nothing" bash -c '! grep -Fq "CALL <wait>" "$1"' _ "$STUB_HC_LOG"
  fi
}
run_kind claude 'EXEC claude <--model> <claude-sonnet-5-5> <--dangerously-skip-permissions> </agentboard-manager>'
run_kind codex 'EXEC codex <--no-daemon> <--model> <gpt-6-sol> <-c> <model_reasoning_effort=medium> <--dangerously-bypass-approvals-and-sandbox> <$agentboard-manager>'
run_kind grok 'EXEC grok <--trust> <--always-approve>'
run_kind codewhale 'EXEC codewhale <--skip-onboarding> <--fresh> <--disable> <subagents> <--approval-policy> <never>'
# exec replaces the launcher: the client is the process the shell started
: >"$CLIENT_LOG"
bash -c 'echo "LAUNCHER $$" >>"$CLIENT_LOG"; exec "$1" codex' _ "$repo/seats/agentboard-manager" >/dev/null 2>&1
is 'client replaces the launcher process (exec, same pid)' bash -c 'l=$(sed -n "s/^LAUNCHER //p" "$1"); c=$(sed -n "s/^PID //p" "$1"); [[ -n $l && $l == "$c" ]]' _ "$CLIENT_LOG"

# the prompt helper retries "still starting", gives up after ten sends, and logs
cwlog=$XDG_RUNTIME_DIR/agentboard-manager-start.log
echo 2 >"$tmp/starting"
: >"$STUB_HC_LOG"; : >"$cwlog"
(export STUB_HC_STARTING="$tmp/starting"; "$repo/seats/agentboard-manager" codewhale) >/dev/null 2>&1
wait_for "$STUB_HC_LOG" '{"started":true}' || true
/usr/bin/sleep 0.2
is 'helper sent three times after two still-starting answers' test "$(grep -Fc 'CALL <send>' "$STUB_HC_LOG")" -eq 3
is 'landed prompt logs no failure' bash -c '! grep -q "did not land" "$1"' _ "$cwlog"
echo 99 >"$tmp/starting"
: >"$STUB_HC_LOG"; : >"$cwlog"
(export STUB_HC_STARTING="$tmp/starting"; "$repo/seats/agentboard-manager" codewhale) >/dev/null 2>&1
wait_for "$cwlog" 'did not land' || true
is 'helper gives up after ten sends' test "$(grep -Fc 'CALL <send>' "$STUB_HC_LOG")" -eq 10
has "$cwlog" 'entry prompt did not land in t4; type /agentboard-manager there' 'unlanded prompt is logged with what to do'
echo 0 >"$tmp/starting"
: >"$STUB_HC_LOG"; : >"$cwlog"
(export STUB_HC_STARTING="$tmp/starting" STUB_HC_NOSTART=1; "$repo/seats/agentboard-manager" grok) >/dev/null 2>&1
wait_for "$cwlog" 'did not land' || true
has "$cwlog" 'did not land' 'started:false is logged as not landed'
is 'started:false is not resent' test "$(grep -Fc 'CALL <send>' "$STUB_HC_LOG")" -eq 1

# herdr refuses
: >"$HERDR_LOG"; : >"$STUB_HC_LOG"
if (export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/agentboard-manager" codewhale) >"$tmp/out" 2>"$tmp/err"; then bad 'codewhale under herdr unexpectedly succeeded'; else ok; fi
has "$tmp/err" 'herdr' 'herdr refusal names herdr'
has "$tmp/err" 'codewhale' 'herdr refusal names codewhale'
is 'herdr refusal never calls herdr' test ! -s "$HERDR_LOG"
is 'herdr refusal never calls hc' test ! -s "$STUB_HC_LOG"

run_neither() {
  : >"$STUB_HC_LOG"
  : >"$HERDR_LOG"
  if (no_hc; export HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/agentboard-manager" claude) >"$tmp/out" 2>"$tmp/neither"; then
    bad 'neither environment unexpectedly succeeded'
  else
    is 'neither environment exits 1' test "$?" -eq 1
  fi
  has "$tmp/neither" 'inside herdr (HERDR_ENV=1) or hc (any HC_* variable set)' 'neither environment says why'
  is 'neither environment starts nothing in hc' bash -c '! grep -Fq "CALL <run>" "$1"' _ "$STUB_HC_LOG"
  is 'neither environment starts nothing in herdr' test ! -s "$HERDR_LOG"
}
run_neither

# any HC_* variable selects hc, but hc needs HC_TERMINAL_ID to know which terminal
: >"$STUB_HC_LOG"; : >"$CLIENT_LOG"
if (no_hc; export HC_OTHER=1; "$repo/seats/agentboard-manager" claude) >"$tmp/out" 2>"$tmp/err"; then bad 'hc without HC_TERMINAL_ID unexpectedly launched'; else ok; fi
has "$tmp/err" 'HC_TERMINAL_ID is required inside hc' 'hc without a terminal id says why'
is 'hc without a terminal id starts nothing' test ! -s "$CLIENT_LOG" -a ! -s "$STUB_HC_LOG"

# herdr branch: HERDR_ENV=1 without the pane and tab ids is refused
run_half_herdr() {
  : >"$HERDR_LOG"
  if (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7; "$repo/seats/agentboard-manager" claude) >"$tmp/out" 2>"$tmp/half"; then
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
  (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/agentboard-manager" "$kind") >"$tmp/out" 2>"$tmp/err"
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
  exact "$HERDR_LOG" 'ENV AGENTBOARD_ACTOR=manager CODEX_HOME= CLAUDE_CONFIG_DIR= GROK_HOME= CODEWHALE_HOME=' "herdr $kind gets the manager actor and no home variable"
  is "herdr $kind never calls hc" test ! -s "$STUB_HC_LOG"
}
run_herdr_kind claude 'CALL <agent> <start> <manager> <--kind> <claude> <--pane> <pane-7> <--> <--model> <claude-sonnet-5-5> <--dangerously-skip-permissions> <--> </agentboard-manager>'
run_herdr_kind codex 'CALL <agent> <start> <manager> <--kind> <codex> <--pane> <pane-7> <--> <--model> <gpt-6-sol> <-c> <model_reasoning_effort=medium> <--dangerously-bypass-approvals-and-sandbox> <--> <$agentboard-manager>'
run_herdr_kind grok 'CALL <agent> <start> <manager> <--kind> <grok> <--pane> <pane-7> <--> <--trust> <--always-approve>'

run_herdr_not_ready() {
  local mode=$1
  : >"$HERDR_LOG"
  : >"$XDG_RUNTIME_DIR/agentboard-manager-start.log"
  (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3 HERDR_MODE=$mode; "$repo/seats/agentboard-manager" claude) >"$tmp/out" 2>&1
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
(export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; "$repo/seats/agentboard-manager" claude) >"$tmp/out" 2>&1
has "$HERDR_LOG" 'CALL <tab> <rename> <tab-3> <manager>' 'both environments pick herdr'
is 'both environments leave hc alone' test ! -s "$STUB_HC_LOG"

run_missing_herdr() {
  if (no_hc; export HERDR_ENV=1 HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3; PATH="$tmp/nohc" "$repo/seats/agentboard-manager" claude) >"$tmp/out" 2>"$tmp/no-herdr"; then
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
  if PATH="$tmp/nohc" "$repo/seats/agentboard-manager" claude >"$tmp/out" 2>"$tmp/no-hc"; then
    bad 'missing hc unexpectedly succeeded'
  else
    is 'missing hc exits 1' test "$?" -eq 1
  fi
  has "$tmp/no-hc" 'required command not found: hc' 'missing hc is reported'
}
run_missing_hc
run_missing_herdr

if "$repo/seats/agentboard-manager" >"$tmp/out" 2>"$tmp/no-kind"; then bad 'no kind unexpectedly succeeded'; else
  is 'no kind exits 1' test "$?" -eq 1
fi
has "$tmp/no-kind" 'usage: agentboard-manager {codex|grok|codewhale|claude}' 'no kind reports usage'
is 'unknown kind exits 1' bash -c '"$1" invalid >/dev/null 2>&1; [[ $? -eq 1 ]]' _ "$repo/seats/agentboard-manager"
if "$repo/seats/agentboard-manager" invalid >"$tmp/out" 2>"$tmp/usage"; then bad 'unknown kind unexpectedly succeeded'; else ok; fi
has "$tmp/usage" 'usage: agentboard-manager {codex|grok|codewhale|claude}' 'unknown kind reports usage'

for flag in -h --help; do
  : >"$STUB_HC_LOG"
  if "$repo/seats/agentboard-manager" "$flag" >"$tmp/help" 2>"$tmp/err"; then ok; else bad "$flag exits non-zero"; fi
  has "$tmp/help" 'usage: agentboard-manager {codex|grok|codewhale|claude}' "$flag prints usage on stdout"
  is "$flag starts nothing" test ! -s "$STUB_HC_LOG"
done

echo "$((n - fail))/$n passed (manager launcher)"
[[ $fail -eq 0 ]]
