#!/usr/bin/env bash
# Surreal backend checks against the real binaries and a throwaway server:
#   (a) parity   - the sqlite and surreal builds print the same transcript
#   (b) claim    - 10 runs x 8 processes x 10 id-less claims over 5 ready tasks: no duplicates
#   (c) ids      - 8 processes x 20 adds: 160 distinct ids, no gaps
#   (d) mutation - with the write lock disabled (b)/(c) must FAIL; restored they pass.
#                  The switch exists in debug builds only, so (d) uses a debug binary.
#   (e) outage   - server killed mid-run: the one-line serve error, not a hang
#   (f) release  - a release binary ignores the lock-disable variable (lock held elsewhere)
#   (g) first use- 8 simultaneous first adds, then 8 first reads, on a fresh board: one schema
# ONLY=<phase> (parity|claim|ids|mutation|outage|release|firstuse) runs one phase after the builds.
# Needs a `surreal` binary. A missing one is a failure, never a skip.
set -u
cd "$(dirname "$0")/.."
export AGENTBOARD_REQUIRE_SURREAL=1
export AGENTBOARD_ACTOR=a
unset AGENTBOARD_UNSAFE_NO_WRITE_LOCK AGENTBOARD_DB AGENTBOARD_SURREAL_URL AGENTBOARD_SURREAL_USER AGENTBOARD_SURREAL_PASS

SURREAL=$(command -v surreal || true)
[ -n "$SURREAL" ] || SURREAL=$HOME/.surrealdb/surreal
if [ ! -x "$SURREAL" ]; then
  echo "FAIL: no surreal binary in PATH or ~/.surrealdb/surreal (AGENTBOARD_REQUIRE_SURREAL=1)"
  exit 1
fi

T=$(mktemp -d "${TMPDIR:-/tmp}/agentboard-surreal.XXXXXX")
SERVER_PID=
stop_server() {
  [ -n "$SERVER_PID" ] || return 0
  kill "$SERVER_PID" 2>/dev/null
  for _ in $(seq 1 50); do kill -0 "$SERVER_PID" 2>/dev/null || break; sleep 0.1; done
  if kill -0 "$SERVER_PID" 2>/dev/null; then kill -9 "$SERVER_PID" 2>/dev/null; echo "FAIL: server $SERVER_PID needed kill -9"; fi
  SERVER_PID=
}
cleanup() { stop_server; rm -rf "$T"; }
trap cleanup EXIT

FAILS=0
check() { # name, status
  if [ "$2" -eq 0 ]; then echo "PASS $1"; else echo "FAIL $1"; FAILS=$((FAILS + 1)); fi
}

echo "== build both feature sets"
cargo build --release >/dev/null 2>&1 || { echo "FAIL: default build"; exit 1; }
cp target/release/agentboard "$T/ab-sqlite"
cargo build --release --no-default-features --features surreal >/dev/null 2>&1 || { echo "FAIL: surreal build"; exit 1; }
cp target/release/agentboard "$T/ab-surreal"
cargo build --release >/dev/null 2>&1 # leave target/release as the default build
# The mutation switch is compiled into debug builds only. Kept under target/ so the
# dependency build is paid once, apart from target/release and target/debug.
DEBUG_TARGET=${AGENTBOARD_SURREAL_DEBUG_TARGET:-$PWD/target/surreal-debug}
echo "(debug surreal build for the mutation phase; the first run compiles all dependencies again)"
CARGO_TARGET_DIR=$DEBUG_TARGET cargo build --no-default-features --features surreal >/dev/null 2>&1 \
  || { echo "FAIL: debug surreal build"; exit 1; }
cp "$DEBUG_TARGET/debug/agentboard" "$T/ab-surreal-debug"
AB_BIN=$T/ab-surreal
want() { [ -z "${ONLY:-}" ] || [ "$ONLY" = "$1" ]; }

# A fresh server on a random loopback port with its own data dir; sets AB (the
# surreal binary with its environment) as a function `ab`.
new_server() {
  stop_server
  local n=$1
  PORT=$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1])')
  DIR=$T/$n
  mkdir -p "$DIR"
  setsid "$SURREAL" db start --no-banner --log warn --bind "127.0.0.1:$PORT" \
    --username root --password root "surrealkv://$DIR/data" >"$DIR/server.log" 2>&1 &
  SERVER_PID=$!
  for _ in $(seq 1 100); do curl -sf "http://127.0.0.1:$PORT/health" >/dev/null && break; sleep 0.1; done
  curl -sf "http://127.0.0.1:$PORT/health" >/dev/null || { echo "FAIL: server $n never came up"; cat "$DIR/server.log"; exit 1; }
  export AGENTBOARD_SURREAL_URL=127.0.0.1:$PORT AGENTBOARD_DB=$DIR/data
}
ab() { "$AB_BIN" "$@"; }
# First use bootstraps the schema. Do it once, alone, so the contention checks measure
# claims and ids rather than a racing first bootstrap.
prime() { ab ls >/dev/null || { echo "FAIL: first use of a fresh board"; exit 1; }; }

# ---- (a) parity ---------------------------------------------------------------
transcript() { # binary
  local B=$1
  $B add "first task" --description "d1" --goal "g1"; $B add "second"
  $B ls; $B claim; $B claim T1; $B get T1
  $B note T1 "n1"; $B ask T1 "q?"; $B get T1 --full
  $B edit T2 --title "second!" --priority 5; $B edit T2 --priority 5; $B edit T1 --title x
  $B add "kid" --parent T1; $B add "kid2" --parent T1 --status review
  $B parent T2 T1; $B parent T1 T1; $B parent T2 none
  $B ls; $B children T1; $B children T2; $B get T1
  $B answer T1 "yes"; $B heartbeat T1; $B report T1 "findings"; $B status T1 review; $B done T1 "ok" --commit abc123
  $B ls; $B ls --status done; $B ls --status bogus; $B get T1 --full
  $B claim T9; $B claim T3; $B status T1 bogus; $B get T7; $B heartbeat T2; $B edit T9 --title z
  $B edit T1; $B add "" 2>&1; $B claim; $B claim
}
norm() { sed -E 's/\b[0-9]+(s|m|h|d)\b/N/g; s/[0-9]{2}:[0-9]{2}/HH:MM/g'; }

if want parity; then
echo "== (a) parity: same inputs, sqlite vs surreal"
export AGENTBOARD_ACTOR=a
(export AGENTBOARD_DB=$T/parity.db; transcript "$T/ab-sqlite" 2>&1 | norm >"$T/parity.sqlite")
new_server parity
transcript "$T/ab-surreal" 2>&1 | norm >"$T/parity.surreal"
if diff "$T/parity.sqlite" "$T/parity.surreal" >"$T/parity.diff"; then
  echo "transcripts identical ($(wc -l <"$T/parity.sqlite") lines)"; check "parity" 0
else
  cat "$T/parity.diff"; check "parity" 1
fi
stop_server
fi

# ---- (b) claim contention -----------------------------------------------------
phase_claim() { # runs
  local runs=$1 bad=0
  new_server "claim.$RANDOM"; prime
  for run in $(seq 1 "$runs"); do
    for i in 1 2 3 4 5; do ab add "r$run-$i" >/dev/null; done
    local pids=
    for p in 1 2 3 4 5 6 7 8; do
      ( for _ in $(seq 1 10); do ab claim 2>&1 | head -1; done >"$DIR/claim.$p" ) &
      pids="$pids $!"
    done
    wait $pids
    local ids claimed distinct dup
    ids=$(cat "$DIR"/claim.? | sed -nE 's/^(T[0-9]+) working.*/\1/p')
    claimed=$(printf '%s\n' "$ids" | sed '/^$/d' | wc -l)
    distinct=$(printf '%s\n' "$ids" | sed '/^$/d' | sort -u | wc -l)
    dup=$((claimed - distinct))
    local tag=ok
    if [ "$dup" -gt 0 ] || [ "$distinct" -ne 5 ]; then tag=FAIL; bad=1; fi
    echo "run=$run claimed=$claimed distinct=$distinct duplicates=$dup $tag"
    rm -f "$DIR"/claim.?
  done
  stop_server
  return $bad
}

# ---- (c) id allocation --------------------------------------------------------
phase_ids() {
  new_server "ids.$RANDOM"; prime
  local pids=
  for p in 1 2 3 4 5 6 7 8; do
    ( for _ in $(seq 1 20); do ab add "p$p" 2>&1 | head -1; done >"$DIR/add.$p" ) &
    pids="$pids $!"
  done
  wait $pids
  local ids total distinct maxid gaps
  ids=$(cat "$DIR"/add.? | sed -nE 's/^T([0-9]+) ready$/\1/p' | sort -n)
  total=$(printf '%s\n' "$ids" | sed '/^$/d' | wc -l)
  distinct=$(printf '%s\n' "$ids" | sed '/^$/d' | sort -u | wc -l)
  maxid=$(printf '%s\n' "$ids" | tail -1)
  gaps=$(comm -13 <(printf '%s\n' "$ids" | sort -u) <(seq 1 "${maxid:-0}" | sort) | wc -l)
  local errs
  errs=$(cat "$DIR"/add.? | sed -E '/^T[0-9]+ ready$/d' | head -3)
  echo "adds=$total distinct=$distinct max=T${maxid:-?} gaps=$gaps (want 160 distinct, max T160, 0 gaps)"
  [ -n "$errs" ] && echo "non-add output: $errs"
  stop_server
  [ "$total" -eq 160 ] && [ "$distinct" -eq 160 ] && [ "${maxid:-0}" -eq 160 ] && [ "$gaps" -eq 0 ]
}


if want claim; then
  echo "== (b) claim contention: 10 runs x 8 processes x 10 id-less claims over 5 ready tasks"
  phase_claim 10; check "claim contention" $?
fi
if want ids; then
  echo "== (c) id allocation: 8 processes x 20 adds"
  phase_ids; check "id allocation" $?
fi

# ---- (d) mutation: lock disabled must fail ------------------------------------
# Debug binary only: a release binary has no switch (see (f)).
if want mutation; then
  echo "== (d) mutation: debug binary with AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1 (lock disabled) must make (b) or (c) FAIL"
  AB_BIN=$T/ab-surreal-debug
  export AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1
  phase_ids; ids_status=$?
  phase_claim 10; claim_status=$?
  unset AGENTBOARD_UNSAFE_NO_WRITE_LOCK
  if [ "$ids_status" -ne 0 ] || [ "$claim_status" -ne 0 ]; then
    echo "mutation detected (ids=$ids_status claim=$claim_status)"; check "mutation caught" 0
  else
    echo "mutation NOT detected: the checks pass with the lock disabled"; check "mutation caught" 1
  fi
  echo "== (d) restored: same debug binary, lock enabled again"
  phase_ids; check "ids restored" $?
  phase_claim 10; check "claims restored" $?
  AB_BIN=$T/ab-surreal
fi

# ---- (e) server killed mid-run ------------------------------------------------
if want outage; then
  echo "== (e) server killed mid-run: one-line serve error, no hang"
  new_server outage
  # PATH and HOME hidden so a dead server cannot be restarted: the binary is unfindable.
  ( for i in $(seq 1 30); do
      timeout 20 env PATH=/nonexistent HOME=/nonexistent "$T/ab-surreal" add "w$i" 2>&1 | head -3
      echo "exit=${PIPESTATUS[0]}"
      sleep 0.05
    done >"$DIR/outage.out" ) &
  WORKER=$!
  sleep 0.5
  kill -9 "$SERVER_PID"; wait "$SERVER_PID" 2>/dev/null; SERVER_PID=
  wait $WORKER
  ok_lines=$(rg -c '^T[0-9]+ ready$' "$DIR/outage.out" || true)
  unavail=$(rg -c '^agentboard: board server unavailable: ' "$DIR/outage.out" || true)
  inflight=$(rg -c '^agentboard: board server at ' "$DIR/outage.out" || true)
  hangs=$(rg -c '^exit=124$' "$DIR/outage.out" || true)
  echo "call outcomes: ${ok_lines:-0} ok, ${unavail:-0} 'board server unavailable', ${inflight:-0} 'board server at ...' (in flight at the kill), ${hangs:-0} hangs (timeout 20s)"
  rg '^agentboard: board server (unavailable:|at ) ?' "$DIR/outage.out" | sed -n '1p;$p'
  [ "${unavail:-0}" -ge 1 ] && [ "${hangs:-0}" -eq 0 ]; check "outage line" $?
  # every failure is exactly one line of a known shape: no stack, no second line
  extra=$(rg -v '^(T[0-9]+ ready|agentboard: board server (unavailable:|at ).*|exit=[0-9]+)$' "$DIR/outage.out" | head -3)
  [ -z "$extra" ]; check "outage output is only ok lines and one-line errors" $?
  [ -z "$extra" ] || echo "unexpected output: $extra"
fi

# ---- (f) a release binary ignores the lock-disable variable --------------------
if want release; then
  echo "== (f) release binary ignores AGENTBOARD_UNSAFE_NO_WRITE_LOCK (lock held by another process)"
  new_server release; prime
  python3 - "$DIR/data/board.lock" >"$DIR/holder.out" <<'EOF' &
import fcntl, sys, time
f = open(sys.argv[1], "a+")
fcntl.flock(f, fcntl.LOCK_EX)
print("held", flush=True)
time.sleep(60)
EOF
  HOLDER=$!
  for _ in $(seq 1 50); do rg -q held "$DIR/holder.out" && break; sleep 0.1; done
  rg -q held "$DIR/holder.out" || { echo "FAIL: lock holder never took the lock"; kill $HOLDER; exit 1; }
  rel=$(env AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1 timeout 30 "$T/ab-surreal" add x 2>&1 | head -1)
  echo "release binary, variable set, lock held elsewhere: $rel"
  dbg=$(env AGENTBOARD_UNSAFE_NO_WRITE_LOCK=1 timeout 30 "$T/ab-surreal-debug" add x 2>&1 | head -1)
  echo "debug binary,   variable set, lock held elsewhere: $dbg"
  kill $HOLDER 2>/dev/null; wait $HOLDER 2>/dev/null
  case "$rel" in "agentboard: board write lock busy: "*) check "release ignores the switch" 0 ;; *) check "release ignores the switch" 1 ;; esac
  case "$dbg" in "T"*" ready") check "debug honours the switch (the mutation check relies on it)" 0 ;; *) check "debug honours the switch (the mutation check relies on it)" 1 ;; esac
  if rg -qa UNSAFE_NO_WRITE_LOCK "$T/ab-surreal"; then check "release binary has no switch string" 1; else check "release binary has no switch string" 0; fi
  stop_server
fi

# ---- (g) first use of a fresh board, concurrently ------------------------------
schema_records() { # how many meta:schema records the server holds
  curl -s -u root:root -H 'surreal-ns: agentboard' -H 'surreal-db: board' \
    -H 'Accept: application/json' -H 'Content-Type: text/plain' \
    --data 'SELECT * FROM meta' "http://127.0.0.1:$PORT/sql" \
    | python3 -c 'import json,sys; print(len(json.load(sys.stdin)[0]["result"]))'
}
first_use() { # verb: add | ls  (8 simultaneous first calls on a fresh board)
  local verb=$1
  new_server "first.$verb"
  local pids=
  for p in 1 2 3 4 5 6 7 8; do
    if [ "$verb" = add ]; then ( ab add "f$p" 2>&1 | head -1 >"$DIR/first.$p" ) & else ( ab ls 2>&1 | head -1 >"$DIR/first.$p" ) & fi
    pids="$pids $!"
  done
  wait $pids
  local ok recs bad=0
  if [ "$verb" = add ]; then
    ok=$(cat "$DIR"/first.? | rg -c '^T[0-9]+ ready$' || true)
    ids=$(cat "$DIR"/first.? | sed -nE 's/^T([0-9]+) ready$/\1/p' | sort -n | tr '\n' ' ')
    [ "${ok:-0}" -eq 8 ] && [ "$ids" = "1 2 3 4 5 6 7 8 " ] || bad=1
    echo "first adds: ${ok:-0}/8 ok, ids: $ids"
  else
    ok=$(cat "$DIR"/first.? | rg -c '^\(no tasks\)$' || true)
    [ "${ok:-0}" -eq 8 ] || bad=1
    echo "first reads: ${ok:-0}/8 ok"
  fi
  recs=$(schema_records)
  echo "meta:schema records: $recs (want 1)"
  [ "$recs" -eq 1 ] || bad=1
  [ "$bad" -ne 0 ] && cat "$DIR"/first.? | sort | uniq -c | head -5
  stop_server
  return $bad
}
if want firstuse; then
  echo "== (g) first use of a fresh board: 8 simultaneous first adds (write sessions) then 8 first reads"
  first_use add; check "first use: 8 simultaneous adds" $?
  first_use ls; check "first use: 8 simultaneous reads" $?
fi

echo
if [ "$FAILS" -eq 0 ]; then echo "surreal.sh: all checks passed"; else echo "surreal.sh: $FAILS check(s) FAILED"; fi
[ "$FAILS" -eq 0 ]
