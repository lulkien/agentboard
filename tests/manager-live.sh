#!/usr/bin/env bash
# One live round against the real herdr: up, status, renew, down on a scratch repo.
# Guarded: runs only with AGENTBOARD_HERDR_LIVE=1. It names its agents scratch-coder and
# scratch-reviewer, so it can never match the manager's own `coder`/`reviewer`, uses a
# throwaway board (board.sh) that the scratch agents are pointed at through --env, and
# only ever closes the pane ids that `up` printed. Every other agent and pane is
# compared before and after and must be identical.
set -u
cd "$(dirname "$0")/.."
[ "${AGENTBOARD_HERDR_LIVE:-}" = 1 ] || { echo "manager-live.sh: skipped (set AGENTBOARD_HERDR_LIVE=1 for a round against the real herdr)"; exit 0; }
B=${AGENTBOARD_TEST_BIN:-$PWD/target/release/agentboard}
S=$(mktemp -d /tmp/agentboard-live.XXXXXX)
source tests/board.sh
use live
export AGENTBOARD_DB=$BOARD AGENTBOARD_ACTOR=live
unset AGENTBOARD_HERDR_SOCK
mkdir "$S/repo" && git -C "$S/repo" init -q
P=scratch-
NAMES=${P}coder,${P}reviewer
PANES=
cleanup() {
  # Only panes this round created, only if still there.
  for p in $PANES; do $B manager pane close "$p" --force >/dev/null 2>&1; done
  board_stop; rm -rf "$S"
}
trap cleanup EXIT
M() { echo "\$ agentboard manager $*"; out=$($B manager "$@" 2>&1); rc=$?; echo "$out"; echo "[exit $rc]"; }
others() { $B manager agent list | awk 'NR>1 && $1 !~ /^scratch-/ {print $1, $2, $5}' | sort; }

before=$(others); panes_before=$($B manager pane list | awk 'NR>1{print $1}' | sort)
env_args=(--env "AGENTBOARD_DB=$BOARD")
[ -z "${AGENTBOARD_SURREAL_URL:-}" ] || env_args+=(--env "AGENTBOARD_SURREAL_URL=$AGENTBOARD_SURREAL_URL")

M up --repo "$S/repo" --label scratch-live --role-name-prefix "$P" "${env_args[@]}"
[ "$rc" -eq 0 ] || { echo "FAIL up"; exit 1; }
PANES=$(grep -o 'pane [^ ]*' <<<"$out" | awk '{print $2}')
echo "created panes: $PANES"
M status
M renew --names "$NAMES"
rn=$rc
M down --names "$NAMES"
dn=$rc
fd=0
if [ "$dn" -ne 0 ]; then
  # The plain down may be refused (exit 3) by a seat that is blocked or still launching;
  # that is the guard working. --force must then really stop it and leave nothing behind.
  M down --names "$NAMES" --force
  fd=$rc
  grep -q 'forced:' <<<"$out" || fd=1
fi
[ "$dn" -eq 0 ] || [ "$dn" -eq 3 ] || fd=1
[ "$fd" -eq 0 ] && PANES=
echo '$ herdr agent list'; herdr agent list
after=$(others); panes_after=$($B manager pane list | awk 'NR>1{print $1}' | sort)
echo '$ agentboard sessions (throwaway board)'; $B sessions
fail=0
[ "$rn" -eq 0 ] || { echo "FAIL renew exit $rn"; fail=1; }
[ "$fd" -eq 0 ] || { echo "FAIL down exit $dn, force $fd"; fail=1; }
[ "$before" = "$after" ] || { echo "FAIL the other agents changed"; diff <(echo "$before") <(echo "$after"); fail=1; }
[ "$panes_before" = "$panes_after" ] || { echo "FAIL panes left behind or lost"; diff <(echo "$panes_before") <(echo "$panes_after"); fail=1; }
[ "$fail" -eq 0 ] && echo "live round ok: nothing of its own left behind (backend: $BACKEND)"
exit "$fail"
