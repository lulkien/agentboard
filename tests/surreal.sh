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
#   (h) epics    - an epic workflow transcript, sqlite vs surreal, identical
#   (i) sessions - register / sessions / prune / unregister against stub tmux and herdr, identical
#   (j) render   - brief / ls filters / get / get --full / clipped titles, sqlite vs surreal, identical
#   (k) dispatch - --print, refusal, --force reset, send, fallback line, warnings, sqlite vs surreal
#   (l) wait     - match, timeout, ask/answer, --open, --epic, --after, bad arguments, identical
#   (m) waitdeath- the server killed under a running wait: one line and exit 1, not a hang
#   (n) cli      - `path` prints the data directory, a sqlite file / `--db <file>` is refused, a bad marker is one line
#   (o) mcp      - serve refuses a wrong-kind board path; the MCP suite on the surreal build; a wait does
#                  not block other tool calls
#   (p) suites   - cli.sh, dispatch.sh, race.sh and wait.sh against BOTH builds (each reads its backend off the binary
#                  and starts its own throwaway servers); mcp_client.py on the sqlite build
# ONLY=<phase> (parity|claim|ids|mutation|outage|release|firstuse|epics|sessions|render|dispatch|wait|waitdeath|cli|mcp|suites)
# runs one phase after the builds.
# Needs a `surreal` binary. A missing one is a failure, never a skip.
set -u
cd "$(dirname "$0")/.."
export AGENTBOARD_REQUIRE_SURREAL=1
export AGENTBOARD_ACTOR=a
unset AGENTBOARD_UNSAFE_NO_WRITE_LOCK AGENTBOARD_DB AGENTBOARD_SURREAL_URL AGENTBOARD_SURREAL_USER AGENTBOARD_SURREAL_PASS

source tests/server.sh
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
cargo build --release --no-default-features --features sqlite >/dev/null 2>&1 || { echo "FAIL: sqlite build"; exit 1; }
cp target/release/agentboard "$T/ab-sqlite"
cargo build --release >/dev/null 2>&1 || { echo "FAIL: default (surreal) build"; exit 1; }
cp target/release/agentboard "$T/ab-surreal"
# target/release is left as the default build: surreal
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
  DIR=$T/$n
  start_server "$DIR"   # tests/server.sh, shared with tests/board.sh
  PORT=$SERVER_PORT
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
  $B answer T1 "yes"; $B heartbeat T1; $B report T1 "findings"; $B diff add T1 --body "parity diff"; $B status T1 review; $B done T1 "ok" --commit abc123
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
    --data 'SELECT * FROM meta:schema' "http://127.0.0.1:$PORT/sql" \
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

# ---- (h) epics: same steps, same text ------------------------------------------
epic_transcript() { # binary
  local B=$1
  $B epic add "Proj" --goal "ship it" --description "d" --path /nonexistent-agentboard-epic
  $B epic add "Two"
  $B epic ls; $B epic ls --status done; $B epic ls --status bogus
  $B add "t1" --epic E1; $B add "t2" --epic E1; $B add "t3" --epic E1; $B add "bad" --epic E9
  $B add "other" --epic E2
  $B get T1; $B epic get E1; $B epic get E2
  $B status T1 done "finished"; $B status T2 dropped "no"; $B ask T3 "q?"
  $B add "kid" --parent T1; $B add "mismatch" --epic E2 --parent T1; $B parent T4 T5
  $B epic get E1; $B epic ls; $B children E1; $B children T1; $B children E2; $B children E9
  $B edit E1 --title "Proj renamed" --goal "ship it now"; $B edit E1 --title "Proj renamed"; $B edit E1
  $B edit E1 --title " "; $B edit E9 --title x; $B edit E1 --path /nonexistent-agentboard-epic2
  $B epic status E1 done "closing"; $B epic ls; $B epic ls --status done; $B epic get E1; $B epic get E1 --full
  $B epic status E1 open; $B epic status E1 bogus; $B epic status E9 done; $B epic get T1; $B epic get E9
  $B get E1; $B children T9; $B get T3
}
if want epics; then
  echo "== (h) epics: sqlite vs surreal transcripts"
  (export AGENTBOARD_DB=$T/epics.db; epic_transcript "$T/ab-sqlite" 2>&1 | norm >"$T/epics.sqlite")
  new_server epics
  epic_transcript "$T/ab-surreal" 2>&1 | norm >"$T/epics.surreal"
  if diff "$T/epics.sqlite" "$T/epics.surreal" >"$T/epics.diff"; then
    echo "epic transcripts identical ($(wc -l <"$T/epics.sqlite") lines)"; check "epics parity" 0
  else
    cat "$T/epics.diff"; check "epics parity" 1
  fi
  # the transcript must really exercise the error paths, not pass by printing nothing
  for want_line in 'no such epic E9' 'is an epic id' 'is a task id' 'bad epic status' 'title cannot be empty' 'nothing to edit' 'tasks: 3 \(' 'done #' 'dropped'; do
    rg -q -- "$want_line" "$T/epics.surreal" || { echo "epic transcript lacks: $want_line"; check "epics transcript coverage" 1; }
  done
  # epic_get counts must agree with children for a mixed set: done / dropped / blocking
  echo "-- E1 counts vs children (T2 done, T3 dropped, T4 blocking, T1 ready; a dropped task leaves the denominator):"
  new_server epics.counts
  ab epic add "Mixed" >/dev/null
  ab add "x1" --epic E1 >/dev/null; ab add "x2" --epic E1 >/dev/null; ab add "x3" --epic E1 >/dev/null; ab add "x4" --epic E1 >/dev/null
  ab status T2 done ok >/dev/null; ab status T3 dropped no >/dev/null; ab ask T4 "q" >/dev/null
  line=$(ab epic ls | head -1); sum=$(ab epic get E1 | rg '^tasks:'); kids=$(ab children E1 | wc -l)
  echo "epic ls : $line"; echo "epic get: $sum"; echo "children: $kids lines"
  case "$line" in *" 1/3 done "*) ok1=0 ;; *) ok1=1 ;; esac
  case "$sum" in "tasks: 4 (1 blocking, 1 ready, 1 done, 1 dropped)") ok2=0 ;; *) ok2=1 ;; esac
  [ "$kids" -eq 4 ]; ok3=$?
  check "epic counts agree with children (dropped stays out of done and the denominator)" $((ok1 + ok2 + ok3))
  stop_server
  # epic ids come from their own counter under the same lock: 8 processes x 5 epic adds
  new_server epics.ids; prime
  pids=
  for p in 1 2 3 4 5 6 7 8; do
    ( for _ in 1 2 3 4 5; do ab epic add "e$p" 2>&1 | head -1; done >"$DIR/epic.$p" ) &
    pids="$pids $!"
  done
  wait $pids
  eids=$(cat "$DIR"/epic.? | sed -nE 's/^E([0-9]+) open$/\1/p' | sort -n)
  etotal=$(printf '%s\n' "$eids" | sed '/^$/d' | wc -l)
  edistinct=$(printf '%s\n' "$eids" | sed '/^$/d' | sort -u | wc -l)
  emax=$(printf '%s\n' "$eids" | tail -1)
  echo "epic adds=$etotal distinct=$edistinct max=E${emax:-?} (want 40 distinct, max E40)"
  [ "$etotal" -eq 40 ] && [ "$edistinct" -eq 40 ] && [ "${emax:-0}" -eq 40 ]; check "epic id allocation" $?
  stop_server
fi

# ---- (i) sessions: stub bridges, same steps, same text ---------------------------
mkstubs() { # dir mode(alive|dead)
  mkdir -p "$1"
  if [ "$2" = alive ]; then
    cat >"$1/tmux" <<'EOF'
#!/bin/sh
case "$1" in
  has-session) [ "$3" = alive-one ] && exit 0; exit 1 ;;
  list-sessions) exit 0 ;;
esac
exit 1
EOF
    cat >"$1/herdr" <<'EOF'
#!/bin/sh
[ "$1 $2" = "agent list" ] && { echo '{"result":{"agents":[{"name":"hz","agent":"claude"}]}}'; exit 0; }
exit 1
EOF
  else
    printf '#!/bin/sh\nexit 1\n' >"$1/tmux"
    printf '#!/bin/sh\nexit 1\n' >"$1/herdr"
  fi
  chmod +x "$1/tmux" "$1/herdr"
}
sessions_transcript() { # binary
  local B=$1
  $B sessions
  $B add "work" >/dev/null
  $B register alive-one --agent claude --task T1; sleep 0.05
  $B register gone-one --agent claude; sleep 0.05
  $B register hz --agent claude --task T1; sleep 0.05
  $B register hz --agent claude; sleep 0.05
  $B register bad --task T9
  $B sessions
  $B sessions prune --dry-run; $B sessions; $B sessions prune; $B sessions
  $B sessions prune
  $B unregister alive-one; $B unregister alive-one; $B unregister nobody; $B sessions
  PATH="$DEAD_STUBS:$PATH" $B sessions prune; PATH="$DEAD_STUBS:$PATH" $B sessions prune --dry-run
  $B sessions
}
if want sessions; then
  echo "== (i) sessions: stub tmux/herdr bridges, sqlite vs surreal transcripts"
  ALIVE_STUBS=$T/stubs-alive; DEAD_STUBS=$T/stubs-dead
  mkstubs "$ALIVE_STUBS" alive; mkstubs "$DEAD_STUBS" dead
  (export AGENTBOARD_DB=$T/sessions.db PATH="$ALIVE_STUBS:$PATH"; sessions_transcript "$T/ab-sqlite" 2>&1 | norm >"$T/sessions.sqlite")
  new_server sessions
  (export PATH="$ALIVE_STUBS:$PATH"; sessions_transcript "$T/ab-surreal" 2>&1 | norm >"$T/sessions.surreal")
  if diff "$T/sessions.sqlite" "$T/sessions.surreal" >"$T/sessions.diff"; then
    echo "session transcripts identical ($(wc -l <"$T/sessions.sqlite") lines)"; check "sessions parity" 0
  else
    cat "$T/sessions.diff"; check "sessions parity" 1
  fi
  # Rows seen in the same millisecond must list in the same order on both backends: by
  # name, byte order (uppercase before lowercase, non-ASCII last). Registered out of order,
  # then `seen` forced equal, so no sleep can hide a missing tiebreak.
  tie_register() { for n in émile mid alpha Zed; do "$1" register "$n" --agent claude >/dev/null; done; }
  tie_order() { "$1" sessions | awk '{print $1}' | tr '\n' ' '; }
  want_order="Zed alpha mid émile "
  (export AGENTBOARD_DB=$T/ties.db PATH="$ALIVE_STUBS:$PATH"
   tie_register "$T/ab-sqlite"
   python3 - "$T/ties.db" <<'EOF'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1]); c.execute("UPDATE sessions SET seen = 1000"); c.commit()
EOF
   tie_order "$T/ab-sqlite" >"$T/ties.sqlite")
  new_server sessions.ties
  (export PATH="$ALIVE_STUBS:$PATH"
   tie_register "$T/ab-surreal"
   curl -s -u root:root -H 'surreal-ns: agentboard' -H 'surreal-db: board' -H 'Accept: application/json' \
     -H 'Content-Type: text/plain' --data 'UPDATE sessions SET seen = 1000' "http://127.0.0.1:$PORT/sql" >/dev/null
   tie_order "$T/ab-surreal" >"$T/ties.surreal")
  echo "forced-tie order, sqlite : $(cat "$T/ties.sqlite")"
  echo "forced-tie order, surreal: $(cat "$T/ties.surreal")"
  [ "$(cat "$T/ties.sqlite")" = "$want_order" ] && [ "$(cat "$T/ties.surreal")" = "$want_order" ]; check "sessions tie order (Zed alpha mid émile) on both backends" $?
  stop_server
  for want_line in 'prune: neither tmux nor herdr answers' 'no registered session "alive-one"' 'no such task T9' 'would remove gone-one' 'removed gone-one' 'nothing to prune' 'hz claude up'; do
    rg -q -- "$want_line" "$T/sessions.surreal" || { echo "session transcript lacks: $want_line"; check "sessions transcript coverage" 1; }
  done
  rg -m3 'prune|would|removed|pruned' "$T/sessions.surreal" | head -5
  stop_server
fi

# ---- (j) render: brief / ls / get, same steps, same text ------------------------
render_transcript() { # binary
  local B=$1
  local LONG BIG
  LONG=$(python3 -c "print('a long title with many words in it ' * 5)")
  BIG=$(python3 -c "print('d' * 4500)")
  $B brief; $B ls
  $B epic add "Render epic" --path /nonexistent-agentboard-render
  $B add "$LONG" --description "$BIG" --goal "goal line one
goal line two"
  $B add "in the epic" --epic E1 --priority 5
  $B add "will block"; $B add "already working" --status working; $B add "to review" --status review
  $B ask T3 "need input?"
  $B claim; $B claim
  $B brief; $B brief --since 1
  $B ls; $B ls --status ready; $B ls --status working; $B ls --status blocking; $B ls --status review
  $B ls --status done; $B ls --assignee a; $B ls --assignee nobody; $B ls --limit 2; $B ls --status bogus
  $B get T1; $B get T1 --full; $B get T2; $B get T3 --full; $B get T4; $B get T9
  $B report T5 "report $BIG"; $B done T4 "all finished"; $B note T1 "a note"
  $B brief; $B get T5 --full; $B get T4; $B children E1; $B children T1; $B epic ls
}
if want render; then
  echo "== (j) render: sqlite vs surreal transcripts"
  (export AGENTBOARD_DB=$T/render.db; render_transcript "$T/ab-sqlite" 2>&1 | norm >"$T/render.sqlite")
  new_server render
  render_transcript "$T/ab-surreal" 2>&1 | norm >"$T/render.surreal"
  if diff "$T/render.sqlite" "$T/render.surreal" >"$T/render.diff"; then
    echo "render transcripts identical ($(wc -l <"$T/render.sqlite") lines)"; check "render parity" 0
  else
    head -40 "$T/render.diff"; check "render parity" 1
  fi
  for want_line in '^[0-9]+ [a-z]{3}, ' 'ON INPUT' '\[input\]' '^EPIC E1' '^done:' 'no such task T9' '^\(no tasks\)' 'bad status' '…' 'events:' 'epic: E1'; do
    rg -q -- "$want_line" "$T/render.surreal" || { echo "render transcript lacks: $want_line"; check "render transcript coverage" 1; }
  done
  stop_server
  # The `done:` list of brief: rows finished in the same millisecond list by id, and the
  # LIMIT of 8 keeps the lowest ids. Ten tasks finished out of order, then updated_at forced
  # equal on both backends, so neither scan order nor timing can supply the order.
  tie_done() { local B=$1 i; for i in $(seq 1 10); do "$B" add "d$i" >/dev/null; done
    for i in 7 3 9 1 5 10 2 8 4 6; do "$B" done "T$i" "r$i" >/dev/null; sleep 0.01; done; }
  brief_done() { "$1" brief | sed -n '/^done:/,$p'; }
  now_ms=$(python3 -c 'import time; print(int(time.time() * 1000))')
  want_done=$(printf 'done:\n'; for i in 1 2 3 4 5 6 7 8; do printf '  T%s r%s\n' "$i" "$i"; done)
  (export AGENTBOARD_DB=$T/tiedone.db
   tie_done "$T/ab-sqlite"
   python3 - "$T/tiedone.db" "$now_ms" <<'EOF'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1]); c.execute("UPDATE tasks SET updated_at = ? WHERE status = 'done'", (int(sys.argv[2]),)); c.commit()
EOF
   brief_done "$T/ab-sqlite" >"$T/tiedone.sqlite")
  new_server render.tie
  tie_done "$T/ab-surreal"
  curl -s -u root:root -H 'surreal-ns: agentboard' -H 'surreal-db: board' -H 'Accept: application/json' \
    -H 'Content-Type: text/plain' --data "UPDATE tasks SET updated_at = $now_ms WHERE status = 'done'" \
    "http://127.0.0.1:$PORT/sql" >/dev/null
  brief_done "$T/ab-surreal" >"$T/tiedone.surreal"
  echo "forced-tie brief done list, sqlite : $(tr '\n' ' ' <"$T/tiedone.sqlite")"
  echo "forced-tie brief done list, surreal: $(tr '\n' ' ' <"$T/tiedone.surreal")"
  [ "$(cat "$T/tiedone.sqlite")" = "$want_done" ] && [ "$(cat "$T/tiedone.surreal")" = "$want_done" ]; check "brief done-list tie order (T1..T8 of 10) on both backends" $?
  stop_server
fi

# ---- (k) dispatch: stub herdr/tmux, same steps, same text ------------------------
mkdispatchstubs() { # dir
  mkdir -p "$1"
  cat >"$1/tmux" <<'EOF'
#!/bin/sh
case "$1" in
  has-session) [ "$3" = alive-one ] && exit 0; exit 1 ;;
  list-sessions) exit 0 ;;
  send-keys) [ "$3" = alive-one ] && exit 0; echo "no such session" >&2; exit 1 ;;
esac
exit 1
EOF
  cat >"$1/herdr" <<'EOF'
#!/bin/sh
case "$1 $2" in
  "agent list") echo '{"result":{"agents":[{"name":"hz","agent":"claude","cwd":"/elsewhere/x","pane_id":"p1"}]}}'; exit 0 ;;
  "agent prompt") [ "$3" = hz ] && exit 0; echo "no such agent" >&2; exit 1 ;;
  "pane send-text"|"pane send-keys") exit 0 ;;
esac
exit 1
EOF
  chmod +x "$1/tmux" "$1/herdr"
}
dispatch_transcript() { # binary
  local B=$1
  $B epic add "Proj" --path /srv/proj
  $B add "t1" --epic E1; $B add "t2"; $B add "t3"
  $B register alive-one --agent claude; $B register gone-one --agent claude; $B register hz --agent claude
  $B dispatch T1 --print; $B get T1
  $B dispatch T1
  $B dispatch T1 --to alive-one; $B get T1
  $B dispatch T1 --to hz
  $B dispatch T1 --to nobody; $B dispatch T1 --to gone-one
  $B edit T2 --assignee gone-one; $B dispatch T2; $B dispatch T2 --to alive-one
  $B edit T3 --assignee alive-one; $B dispatch T3; $B get T3
  $B claim T1; $B dispatch T1; $B dispatch T1 --print; $B dispatch T1 --force --to alive-one; $B get T1
  $B dispatch T9; $B dispatch T2 --force --print
}
if want dispatch; then
  echo "== (k) dispatch: stub tmux/herdr, sqlite vs surreal transcripts"
  D_STUBS=$T/stubs-dispatch; mkdispatchstubs "$D_STUBS"
  (export AGENTBOARD_DB=$T/dispatch.db PATH="$D_STUBS:$PATH"; dispatch_transcript "$T/ab-sqlite" 2>&1 | norm >"$T/dispatch.sqlite")
  new_server dispatch
  (export PATH="$D_STUBS:$PATH"; dispatch_transcript "$T/ab-surreal" 2>&1 | norm >"$T/dispatch.surreal")
  if diff "$T/dispatch.sqlite" "$T/dispatch.surreal" >"$T/dispatch.diff"; then
    echo "dispatch transcripts identical ($(wc -l <"$T/dispatch.sqlite") lines)"; check "dispatch parity" 0
  else
    head -40 "$T/dispatch.diff"; check "dispatch parity" 1
  fi
  for want_line in 'dispatched to hz via herdr' 'dispatched to alive-one via tmux \(herdr: no such agent\)' '^warning: T1 belongs to epic E1' 'has no registered assignee' 'no registered session "nobody"' 'registered but not live' 'row is stale' '^--force: T1 working -> ready' 'is working, not ready' 'Use the agentboard MCP tools' 'dispatch N: alive-one via tmux' 'no such task T9'; do
    rg -q -- "$want_line" "$T/dispatch.surreal" || { echo "dispatch transcript lacks: $want_line"; check "dispatch transcript coverage" 1; }
  done
  stop_server
fi

# ---- (l) wait: same steps, same text, shapes and markers only --------------------
wait_transcript() { # binary
  local B=$1 BIG
  BIG=$(python3 -c "print('tail text ' * 120)")
  $B add "w1"; $B add "w2"; $B epic add "WE"
  $B add "in epic" --epic E1; $B add "long one" --description "$BIG"; $B add "sub" --parent T3
  # already matching: returns at once, with the latest event as context
  $B wait T1 --status ready --timeout 5; echo "exit=$?"
  $B wait T1 T2 --status ready,done --timeout 5; echo "exit=$?"
  # nothing matches: a status line per task and the tail of the freshest, clipped with a marker
  $B wait T4 T1 --kind done --timeout 1; echo "exit=$?"
  # an ask / answer round trip, each delivered as a match
  $B claim T1 >/dev/null
  ( sleep 0.7; $B ask T1 "which one?" >/dev/null ) & $B wait T1 --kind ask --timeout 30; echo "exit=$?"; wait
  ( sleep 0.7; $B answer T1 "the first one" >/dev/null ) & $B wait T1 --kind answer --timeout 30; echo "exit=$?"; wait
  # --status and --kind together
  ( sleep 0.7; $B diff add T1 --body "fixture diff" >/dev/null; $B status T1 review "done?" >/dev/null ) & $B wait T1 --status review --kind review --timeout 30; echo "exit=$?"; wait
  # --open picks up a task created during the wait
  ( sleep 0.7; $B add "created mid-wait" >/dev/null ) & $B wait --open --kind add --timeout 30; echo "exit=$?"; wait
  # --epic: the tasks of the epic, subtasks included
  ( sleep 0.7; $B note T5 "epic subtask note" >/dev/null ) & $B wait --epic E1 --kind note --timeout 30; echo "exit=$?"; wait
  # --after: only events past a seq
  $B wait T1 --after 0 --kind claim --timeout 5; echo "exit=$?"
  $B wait T1 --after 999 --kind claim --timeout 1; echo "exit=$?"
  # bad arguments: exit 1 and the same stderr, never a filter that matches nothing
  $B wait T1 --status bogus; echo "exit=$?"
  $B wait T1 --kind reclaim; echo "exit=$?"
  $B wait T1 --kind ""; echo "exit=$?"
  $B wait T1 --status " , "; echo "exit=$?"
  $B wait; echo "exit=$?"
  $B wait T99 --timeout 1; echo "exit=$?"
  $B wait --epic E9 --timeout 1; echo "exit=$?"
  $B epic add "Empty" >/dev/null; $B wait --epic E2 --timeout 1; echo "exit=$?"
  $B wait E1 --timeout 1; echo "exit=$?"
}
if want wait; then
  echo "== (l) wait: sqlite vs surreal transcripts"
  (export AGENTBOARD_DB=$T/wait.db; wait_transcript "$T/ab-sqlite" 2>&1 | norm >"$T/wait.sqlite")
  new_server wait
  wait_transcript "$T/ab-surreal" 2>&1 | norm >"$T/wait.surreal"
  if diff "$T/wait.sqlite" "$T/wait.surreal" >"$T/wait.diff"; then
    echo "wait transcripts identical ($(wc -l <"$T/wait.sqlite") lines)"; check "wait parity" 0
  else
    head -40 "$T/wait.diff"; check "wait parity" 1
  fi
  for want_line in '^matched: 1 of 1 watched' '^last: #' '^timeout: nothing matched after N \(2 watched\)' '^last \(T[0-9]+\): ' 'truncated' 'block: which one\?' 'answer: the first one' 'review: done\?' 'add: created mid-wait' 'note: epic subtask note' '^exit=0$' '^exit=1$' '^exit=3$' 'wait: bad status "bogus"' 'wait: bad kind "reclaim"' 'wait: empty --kind' 'wait: empty --status' 'wait: name tasks' 'no such task T99' 'no such epic E9' 'match no tasks'; do
    rg -q -- "$want_line" "$T/wait.surreal" || { echo "wait transcript lacks: $want_line"; check "wait transcript coverage" 1; }
  done
  stop_server
fi

# ---- (m) the server dies under a running wait ------------------------------------
if want waitdeath; then
  echo "== (m) server killed under a running wait: the wait ends with one line (bounded, not a hang)"
  new_server waitdeath; prime
  ab add "w" >/dev/null
  start=$(date +%s)
  ( timeout 60 "$T/ab-surreal" wait --open --status done --timeout 30 >"$DIR/wd.out" 2>&1; echo "rc=$?" >"$DIR/wd.rc" ) &
  WPID=$!
  sleep 1.5
  kill -9 "$SERVER_PID"; wait "$SERVER_PID" 2>/dev/null; SERVER_PID=
  wait $WPID
  elapsed=$(( $(date +%s) - start ))
  rc=$(cat "$DIR/wd.rc"); lines=$(wc -l <"$DIR/wd.out")
  echo "wait ended with $rc after about ${elapsed}s (its own timeout was 30 s); output ($lines line): $(head -c 200 "$DIR/wd.out")"
  # not 124 (killed by the outer timeout), exit 1, exactly one line, an agentboard error
  [ "$rc" = "rc=1" ] && [ "$lines" -eq 1 ] && rg -q '^agentboard: board (server|database)' "$DIR/wd.out" && [ "$elapsed" -lt 25 ]
  check "wait ends with one line when the server dies" $?
  # and no server of ours is left running (the process is named surreal-db on this box)
  if ps -eo pid,args | rg -F "$DIR/data" | rg -v ' rg ' | rg -q surreal; then check "no server left behind" 1; else check "no server left behind" 0; fi
fi

# ---- (n) the CLI adapter in the surreal build --------------------------------------
if want cli; then
  echo "== (n) cli: path, wrong-kind board paths, schema marker"
  clifail=0
  # `path`: the surreal data directory, resolved like the store does; it creates nothing
  got=$(env -u AGENTBOARD_DB XDG_STATE_HOME="$T/cli-state" "$T/ab-surreal" path)
  echo "path (default):        $got"
  [ "$got" = "$T/cli-state/agentboard/surreal" ] && [ ! -e "$T/cli-state" ] || clifail=1
  got=$(AGENTBOARD_DB="$T/some-dir" "$T/ab-surreal" path); echo "path (AGENTBOARD_DB):  $got"
  [ "$got" = "$T/some-dir" ] || clifail=1
  got=$("$T/ab-surreal" --db "$T/other-dir" path); echo "path (--db):           $got"
  [ "$got" = "$T/other-dir" ] || clifail=1
  got=$(env -u AGENTBOARD_DB XDG_STATE_HOME="$T/cli-state" "$T/ab-sqlite" path); echo "path (sqlite build):   $got"
  [ "$got" = "$T/cli-state/agentboard/board.db" ] || clifail=1
  # a SQLite file where the data directory belongs: refused, nothing else printed, nothing created
  AGENTBOARD_DB="$T/cli-sqlite.db" "$T/ab-sqlite" add x >/dev/null
  want_line="agentboard: path '$T/cli-sqlite.db' is a SQLite file, not a SurrealDB directory. Move it aside (mv '$T/cli-sqlite.db' '$T/cli-sqlite.db.old') and retry."
  for how in env flag; do
    if [ "$how" = env ]; then out=$(AGENTBOARD_DB="$T/cli-sqlite.db" "$T/ab-surreal" ls 2>&1); else out=$(env -u AGENTBOARD_DB "$T/ab-surreal" --db "$T/cli-sqlite.db" ls 2>&1); fi
    rc=$?
    echo "sqlite file via $how: rc=$rc: $out" | cut -c1-200
    [ "$rc" = 1 ] && [ "$out" = "$want_line" ] || clifail=1
  done
  [ -f "$T/cli-sqlite.db" ] && [ ! -e "$T/cli-sqlite.db.old" ] && [ -z "$(ls -A "$T" | rg '^cli-sqlite.db/')" ] || clifail=1
  # the reverse: a directory given to the sqlite build
  mkdir -p "$T/cli-a-dir"
  out=$(AGENTBOARD_DB="$T/cli-a-dir" "$T/ab-sqlite" ls 2>&1); rc=$?
  echo "directory to the sqlite build: rc=$rc: $out" | cut -c1-200
  [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | wc -l)" -eq 1 ] && printf '%s' "$out" | rg -q "is a directory, not a SQLite board file" || clifail=1
  # a wrong schema marker through the CLI: the store's one line, unchanged
  new_server cli; prime
  curl -s -u root:root -H 'surreal-ns: agentboard' -H 'surreal-db: board' -H 'Accept: application/json' \
    -H 'Content-Type: text/plain' --data 'UPDATE meta:schema SET version = 99' "http://127.0.0.1:$PORT/sql" >/dev/null
  out=$(ab ls 2>&1); rc=$?
  echo "wrong marker: rc=$rc: $out" | cut -c1-200
  [ "$rc" = 1 ] && [ "$(printf '%s\n' "$out" | wc -l)" -eq 1 ] && printf '%s' "$out" | rg -q '^agentboard: board schema mismatch: found schema version 99, want version 2\.' || clifail=1
  stop_server
  check "cli adapter: path, wrong-kind refusals, marker line" $clifail
fi

# ---- (o) the rmcp adapter on the surreal build -------------------------------------
if want mcp; then
  echo "== (o) mcp: serve's wrong-kind check, the MCP suite on the surreal build, a wait that does not block"
  # serve refuses a board path of the wrong kind before the transport starts: one stderr line, rc 1,
  # nothing on stdout (no protocol frame), no tool call needed.
  AGENTBOARD_DB="$T/mcp-sqlite.db" "$T/ab-sqlite" add x >/dev/null
  mkdir -p "$T/mcp-a-dir"
  serve_fail=0
  for how in serve default; do
    if [ "$how" = serve ]; then args=(serve); else args=(); fi
    AGENTBOARD_DB="$T/mcp-sqlite.db" "$T/ab-surreal" "${args[@]}" </dev/null >"$T/serve.out" 2>"$T/serve.err"; rc=$?
    echo "surreal build ($how) on a sqlite file: rc=$rc stdout=$(wc -c <"$T/serve.out") bytes: $(cut -c1-130 "$T/serve.err")"
    [ "$rc" = 1 ] && [ ! -s "$T/serve.out" ] && [ "$(wc -l <"$T/serve.err")" -eq 1 ] \
      && rg -q "^agentboard: path '$T/mcp-sqlite.db' is a SQLite file, not a SurrealDB directory\." "$T/serve.err" || serve_fail=1
    AGENTBOARD_DB="$T/mcp-a-dir" "$T/ab-sqlite" "${args[@]}" </dev/null >"$T/serve.out" 2>"$T/serve.err"; rc=$?
    echo "sqlite build  ($how) on a directory: rc=$rc stdout=$(wc -c <"$T/serve.out") bytes: $(cut -c1-130 "$T/serve.err")"
    [ "$rc" = 1 ] && [ ! -s "$T/serve.out" ] && [ "$(wc -l <"$T/serve.err")" -eq 1 ] \
      && rg -q "^agentboard: path '$T/mcp-a-dir' is a directory, not a SQLite board file\." "$T/serve.err" || serve_fail=1
  done
  check "serve refuses a wrong-kind board path (stderr line, rc 1, empty stdout)" $serve_fail

  # the whole MCP suite against the surreal build: a fresh server and a fresh data directory
  new_server mcp
  out=$(AGENTBOARD_TEST_BIN="$T/ab-surreal" AGENTBOARD_TEST_DB="$DIR/data" python3 tests/mcp_client.py 2>&1)
  line=$(printf '%s\n' "$out" | rg '^[0-9]+/[0-9]+ checks passed \(backend: surreal\)$' | tail -1)
  echo "MCP suite on the surreal build: ${line:-no summary line}"
  printf '%s\n' "$out" | rg '^FAIL' | head -5
  n=${line%%/*}; m=${line#*/}; m=${m%% *}
  [ -n "$line" ] && [ "$n" = "$m" ] && [ "$n" -ge 121 ]; check "MCP suite passes on the surreal build" $?
  stop_server

  # a task_wait in flight does not block the server: another tool call is answered meanwhile
  new_server mcp.wait
  cat >"$T/mcp_wait.py" <<'EOF'
import json, os, subprocess, sys, time
p = subprocess.Popen([os.environ["BIN"]], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
                     env={**os.environ, "AGENTBOARD_ACTOR": "c"})
def send(o): p.stdin.write(json.dumps(o) + "\n"); p.stdin.flush()
def recv(): return json.loads(p.stdout.readline())
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}})
recv()
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
def call(i, name, **a): send({"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": name, "arguments": a}})
call(2, "task_add", title="waited on"); r = recv(); assert r["id"] == 2, r
t0 = time.time()
call(3, "task_wait", id="T1", kind="done", timeout_s=4)      # nothing will happen: a 4 s timeout
time.sleep(0.5)
call(4, "task_list")
first = recv()
meanwhile = time.time() - t0
second = recv()
total = time.time() - t0
print(f"first reply id={first['id']} after {meanwhile:.1f}s; second reply id={second['id']} after {total:.1f}s")
print("first :", first["result"]["content"][0]["text"].splitlines()[0])
print("second:", second["result"]["content"][0]["text"].splitlines()[0])
p.stdin.close(); p.wait(timeout=5)
ok = first["id"] == 4 and second["id"] == 3 and meanwhile < 3.0 and total >= 3.5 \
     and second["result"]["content"][0]["text"].startswith("timeout:")
sys.exit(0 if ok else 1)
EOF
  BIN="$T/ab-surreal" python3 "$T/mcp_wait.py"; check "a pending task_wait does not block another tool call (surreal build)" $?
  stop_server

  # writes parked on the board write lock do not starve a read: two runtime workers, the lock held
  # elsewhere, 8 task_add then a task_list on one stdio session. Every store call runs on the blocking
  # pool, so the read is answered at once and the writes finish only after the lock frees.
  new_server mcp.starve; prime
  cat >"$T/mcp_starve.py" <<'EOF'
import fcntl, json, os, subprocess, sys, time
HOLD = 4.5
p = subprocess.Popen([os.environ["BIN"]], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, bufsize=1,
                     env={**os.environ, "AGENTBOARD_ACTOR": "c", "TOKIO_WORKER_THREADS": "2"})
def send(o): p.stdin.write(json.dumps(o) + "\n"); p.stdin.flush()
def recv(): return json.loads(p.stdout.readline())
send({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}})
recv()
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
def call(i, name, **a): send({"jsonrpc": "2.0", "id": i, "method": "tools/call", "params": {"name": name, "arguments": a}})
call(2, "task_list"); recv()                      # warm: the server is up and answering
lock = open(os.path.join(os.environ["AGENTBOARD_DB"], "board.lock"), "a+")
fcntl.flock(lock, fcntl.LOCK_EX)
t0 = time.time()
for i in range(8):
    call(10 + i, "task_add", title=f"starve {i}")
call(30, "task_list")
at, got = {}, {}
while 30 not in at:
    r = recv(); at[r["id"]] = time.time() - t0; got[r["id"]] = r
read_at = at[30]
time.sleep(max(0.0, HOLD - (time.time() - t0)))
fcntl.flock(lock, fcntl.LOCK_UN)
while len(at) < 9:
    r = recv(); at[r["id"]] = time.time() - t0; got[r["id"]] = r
p.stdin.close(); p.wait(timeout=5)
writes = [at[10 + i] for i in range(8)]
print(f"task_list answered at {read_at:.2f}s (lock held {HOLD}s); 8 task_add answered at {min(writes):.2f}-{max(writes):.2f}s")
ok = read_at < 1.0 and min(writes) >= HOLD - 0.5 and not any(got[10 + i]["result"].get("isError") for i in range(8))
sys.exit(0 if ok else 1)
EOF
  BIN="$T/ab-surreal" python3 "$T/mcp_starve.py"; check "lock-blocked writes do not starve a read (2 workers, surreal build)" $?
  stop_server
fi

# ---- (p) the shell suites on both builds ------------------------------------------------
if want suites; then
  echo "== (p) suites: cli.sh, dispatch.sh, race.sh and wait.sh on both builds, mcp_client.py on the sqlite build"
  # The summary line names the backend the suite detected on its binary; it must be the
  # one this phase meant to test, or a swapped binary would still print PASS.
  suite_line() { # label, expected backend (surreal|sqlite), command...
    local label=$1 want_backend=$2; shift 2
    local out line n m
    out=$("$@" 2>&1)
    line=$(printf '%s\n' "$out" | rg "^[0-9]+/[0-9]+ (passed|checks passed) \\(backend: $want_backend\\)\$" | tail -1)
    echo "$label: ${line:-no summary line}"
    printf '%s\n' "$out" | rg '^FAIL' | head -5
    n=${line%%/*}; m=${line#*/}; m=${m%% *}
    [ -n "$line" ] && [ "$n" = "$m" ]
  }
  for kind in surreal sqlite; do
    suite_line "cli.sh on the $kind build" $kind env AGENTBOARD_TEST_BIN="$T/ab-$kind" bash tests/cli.sh;       check "cli.sh passes on the $kind build" $?
    suite_line "dispatch.sh on the $kind build" $kind env AGENTBOARD_TEST_BIN="$T/ab-$kind" bash tests/dispatch.sh; check "dispatch.sh passes on the $kind build" $?
    out=$(AGENTBOARD_TEST_BIN="$T/ab-$kind" bash tests/race.sh 2>&1); echo "race.sh on the $kind build: $(printf '%s\n' "$out" | tail -1)"
    printf '%s\n' "$out" | tail -1 | rg -q "^claims=80 unique=80 .*\(backend: $kind\)\$"; check "race.sh: 80 unique claims on the $kind build" $?
    out=$(AGENTBOARD_TEST_BIN="$T/ab-$kind" bash tests/wait.sh 2>&1); echo "wait.sh on the $kind build: $(printf '%s\n' "$out" | tail -2 | tr '\n' ' ')"
    printf '%s\n' "$out" | rg -q "^matched:" && printf '%s\n' "$out" | tail -1 | rg -q "^elapsed_ms=[0-9]+ \(backend: $kind\)\$"; check "wait.sh returns on the event on the $kind build" $?
  done
  suite_line "mcp_client.py on the sqlite build" sqlite env AGENTBOARD_TEST_BIN="$T/ab-sqlite" python3 tests/mcp_client.py
  check "MCP suite passes on the sqlite build" $?
fi

echo
if [ "$FAILS" -eq 0 ]; then echo "surreal.sh: all checks passed"; else echo "surreal.sh: $FAILS check(s) FAILED"; fi
[ "$FAILS" -eq 0 ]
