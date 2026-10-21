# Sourced by board.sh and surreal.sh: the one way a suite starts a throwaway board server.
# start_server DIR serves DIR/data on a random loopback port (never the default one) and
# sets SERVER_PORT and SERVER_PID. Exits the calling script when it cannot.
start_server() {
  local dir=$1 surreal
  surreal=$(command -v surreal || true)
  [ -n "$surreal" ] || surreal=$HOME/.surrealdb/surreal
  [ -x "$surreal" ] || { echo "FAIL: no surreal binary in PATH or ~/.surrealdb/surreal"; exit 1; }
  SERVER_PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
  mkdir -p "$dir"
  setsid "$surreal" db start --no-banner --log warn --bind "127.0.0.1:$SERVER_PORT" \
    --username root --password root "surrealkv://$dir/data" >"$dir/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 100); do curl -sf "http://127.0.0.1:$SERVER_PORT/health" >/dev/null && break; sleep 0.1; done
  curl -sf "http://127.0.0.1:$SERVER_PORT/health" >/dev/null || { echo "FAIL: board server in $dir never came up"; cat "$dir/server.log"; exit 1; }
}
