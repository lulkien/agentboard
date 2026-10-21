# Sourced by cli.sh, dispatch.sh, race.sh and wait.sh. Wants S (scratch dir) and B (binary under test).
# The same suite runs against either build: BACKEND is read off the binary's own
# --help, so a suite cannot quietly run one backend while claiming the other.
# `use NAME` names a throwaway board: BOARD is its --db value. A surreal board is a
# server of its own on a random loopback port (one URL is one namespace, so two boards
# need two servers), a sqlite board is a file. No suite inherits a board from the caller's
# environment: both variables are cleared here, so the default server (the live board on a
# box that runs one) is unreachable until `use` points the environment somewhere else.
unset AGENTBOARD_DB AGENTBOARD_SURREAL_URL
source "$(dirname "${BASH_SOURCE[0]}")/server.sh"
[ -x "$B" ] || { echo "FAIL: no binary at $B; build it (cargo build --release) or set AGENTBOARD_TEST_BIN=<path>"; exit 1; }
if "$B" --help 2>&1 | grep -q 'Data directory of the local board server'; then BACKEND=surreal; else BACKEND=sqlite; fi
declare -A BOARD_URL=()
BOARD_PIDS=()
BOARD=
use() {
  if [ "$BACKEND" = sqlite ]; then BOARD=$S/$1.db; return; fi
  if [ -z "${BOARD_URL[$1]:-}" ]; then
    start_server "$S/$1"
    BOARD_PIDS+=("$SERVER_PID")
    BOARD_URL[$1]=127.0.0.1:$SERVER_PORT
  fi
  export AGENTBOARD_SURREAL_URL=${BOARD_URL[$1]}
  BOARD=$S/$1/data
}
board_stop() { [ "${#BOARD_PIDS[@]}" -eq 0 ] || kill "${BOARD_PIDS[@]}" 2>/dev/null; return 0; }
