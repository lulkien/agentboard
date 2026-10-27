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

mkdir "$tmp/bin"
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
export PATH="$tmp/bin:$PATH" HERDR_PANE_ID=pane-7 HERDR_TAB_ID=tab-3 HERDR_LOG="$tmp/herdr.log" XDG_RUNTIME_DIR="$tmp" HERDR_MODE=ready
export HOME="$tmp/home"

run_kind() {
  local kind=$1 expected=$2
  : >"$HERDR_LOG"
  "$repo/seats/start-agentboard-manager" "$kind"
  for _ in {1..50}; do
    grep -qF 'CALL <agent> <start> <manager>' "$HERDR_LOG" && break
    sleep 0.02
  done
  is "$kind command starts manager" grep -Fq "$expected" "$HERDR_LOG"
  is "$kind renames tab" grep -Fq 'CALL <tab> <rename> <tab-3> <manager>' "$HERDR_LOG"
  if [[ $kind == grok || $kind == hermes ]]; then
    for _ in {1..50}; do
      grep -qF 'CALL <agent> <prompt>' "$HERDR_LOG" && break
      sleep 0.02
    done
    has "$HERDR_LOG" 'CALL <agent> <prompt> <manager> </agentboard-manager> <--wait>' "$kind sends the first prompt"
  fi
}
repo=$(pwd)
run_kind claude 'CALL <agent> <start> <manager> <--kind> <claude> <--pane> <pane-7> <--> <--model> <claude-sonnet-5-5> <--mcp-config> <'$HOME'/.claude/manager-mcp.json> <--dangerously-skip-permissions> <--> </agentboard-manager>'
run_kind codex 'CALL <agent> <start> <manager> <--kind> <codex> <--pane> <pane-7> <--> <--profile> <manager> <--model> <gpt-6-luna> <--> <$agentboard-manager>'
run_kind grok 'CALL <agent> <start> <manager> <--kind> <grok> <--pane> <pane-7> <--> <--trust> <--always-approve>'
run_kind hermes 'CALL <agent> <start> <manager> <--kind> <hermes> <--pane> <pane-7> <--> <--yolo>'

run_not_ready() {
  local mode=$1
  : >"$HERDR_LOG"
  : >"$XDG_RUNTIME_DIR/agentboard-manager-start.log"
  HERDR_MODE=$mode "$repo/seats/start-agentboard-manager" claude
  for _ in {1..100}; do
    grep -qF 'timed out waiting for shell prompt' "$XDG_RUNTIME_DIR/agentboard-manager-start.log" && break
    /usr/bin/sleep 0.02
  done
  /usr/bin/sleep 0.02
  is "$mode info does not start agent" bash -c '! grep -Fq "CALL <agent> <start>" "$1"' _ "$HERDR_LOG"
  has "$XDG_RUNTIME_DIR/agentboard-manager-start.log" 'timed out waiting for shell prompt' "$mode info times out"
}
run_not_ready missing
run_not_ready mismatch

if "$repo/seats/start-agentboard-manager" >"$tmp/out" 2>"$tmp/no-kind"; then bad 'no kind unexpectedly succeeded'; else
  is 'no kind exits 1' test "$?" -eq 1
fi
has "$tmp/no-kind" 'usage: start-agentboard-manager {claude|codex|grok|hermes}' 'no kind reports usage'
is 'unknown kind exits 1' bash -c '"$1" invalid >/dev/null 2>&1; [[ $? -eq 1 ]]' _ "$repo/seats/start-agentboard-manager"
if "$repo/seats/start-agentboard-manager" invalid >"$tmp/out" 2>"$tmp/usage"; then bad 'unknown kind unexpectedly succeeded'; else ok; fi
has "$tmp/usage" 'usage: start-agentboard-manager {claude|codex|grok|hermes}' 'unknown kind reports usage'

echo "$((n - fail))/$n passed (manager launcher)"
[[ $fail -eq 0 ]]
