# Sourced by cli.sh and dispatch.sh. Wants S (scratch dir) and B (binary under test).
# The same suite runs against either build: BACKEND is read off the binary's own
# --help, so a suite cannot quietly run one backend while claiming the other.
# `use NAME` names a throwaway board: BOARD is its --db value. A surreal board is a
# server of its own on a random loopback port (one URL is one namespace, so two boards
# need two servers), a sqlite board is a file. The live board and its port are never touched.
if "$B" --help 2>&1 | grep -q 'Data directory of the local board server'; then BACKEND=surreal; else BACKEND=sqlite; fi
declare -A BOARD_URL=()
BOARD_PIDS=()
BOARD=
use() {
  if [ "$BACKEND" = sqlite ]; then BOARD=$S/$1.db; return; fi
  local dir=$S/$1 port
  if [ -z "${BOARD_URL[$1]:-}" ]; then
    local surreal
    surreal=$(command -v surreal || true)
    [ -n "$surreal" ] || surreal=$HOME/.surrealdb/surreal
    [ -x "$surreal" ] || { echo "FAIL: no surreal binary in PATH or ~/.surrealdb/surreal"; exit 1; }
    port=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
    mkdir -p "$dir"
    setsid "$surreal" db start --no-banner --log warn --bind "127.0.0.1:$port" \
      --username root --password root "surrealkv://$dir/data" >"$dir/server.log" 2>&1 &
    BOARD_PIDS+=($!)
    for _ in $(seq 1 100); do curl -sf "http://127.0.0.1:$port/health" >/dev/null && break; sleep 0.1; done
    curl -sf "http://127.0.0.1:$port/health" >/dev/null || { echo "FAIL: board server $1 never came up"; cat "$dir/server.log"; exit 1; }
    BOARD_URL[$1]=127.0.0.1:$port
  fi
  export AGENTBOARD_SURREAL_URL=${BOARD_URL[$1]}
  BOARD=$dir/data
}
board_stop() { [ "${#BOARD_PIDS[@]}" -eq 0 ] || kill "${BOARD_PIDS[@]}" 2>/dev/null; return 0; }
