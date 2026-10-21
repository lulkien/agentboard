#!/usr/bin/env bash
# Concurrency probe: N workers drain a queue of tasks. Proves no task is
# claimed twice and none is lost. Runs on a throwaway board (tests/board.sh),
# never the default one; AGENTBOARD_TEST_BIN picks the binary.
set -u
cd "$(dirname "$0")/.."
B=${AGENTBOARD_TEST_BIN:-$PWD/target/release/agentboard}
S=$(mktemp -d /tmp/agentboard-race.XXXXXX)
source tests/board.sh
trap 'board_stop; rm -rf "$S"' EXIT
use race
export AGENTBOARD_DB=$BOARD
echo "backend: $BACKEND, board: $BOARD${AGENTBOARD_SURREAL_URL:+ at $AGENTBOARD_SURREAL_URL}"
W=8
# The first add proves the board answers; without it 80 failing adds would each wait out a connect timeout.
$B add "task 1" >/dev/null || { echo "FAIL race.sh: the board did not answer the first add (backend: $BACKEND)"; exit 1; }
for i in $(seq 2 80); do $B add "task $i" >/dev/null; done

# A worker's only answers are a won task ("T14 working ...") or "no ready tasks". Anything
# else is a backend error (server gone, connection refused, schema refusal): the worker
# says which attempt hit it and fails, so the suite fails instead of spinning. Bounded
# three ways: every claim by a timeout, every worker by an attempt cap and a wall-clock
# deadline, and the other workers by the abort flag the first failure raises.
MAX_ATTEMPTS=${RACE_MAX_ATTEMPTS:-120}
DEADLINE=${RACE_DEADLINE_S:-120}
worker() {
  local w=$1 out=$2 attempt=0 t
  : > "$out"
  while :; do
    attempt=$((attempt + 1))
    if [ -e "$S/abort" ]; then return 1; fi
    if [ "$attempt" -gt "$MAX_ATTEMPTS" ] || [ "$SECONDS" -ge "$DEADLINE" ]; then
      echo "race.sh: worker w$w gave up at attempt $attempt after ${SECONDS}s (cap $MAX_ATTEMPTS attempts / ${DEADLINE}s) without 'no ready tasks'" >"$S/fail.$w"; : >"$S/abort"; return 1
    fi
    t=$(AGENTBOARD_ACTOR="w$w" timeout 30 $B claim 2>&1 | head -1)
    if [ "$t" = "no ready tasks" ]; then break; fi
    if ! [[ "$t" =~ ^T[0-9]+\ working ]]; then
      echo "race.sh: worker w$w attempt $attempt: backend error instead of a claim: ${t:-<no output, claim timed out or died>}" >"$S/fail.$w"; : >"$S/abort"; return 1
    fi
    echo "$t" >> "$out"
    # every few claims, finish one so the queue keeps moving
    local id=${t%% *}
    if [ "${id#T}" -le 8 ] 2>/dev/null; then
      $B done "$id" "finished by w$w" >/dev/null
    fi
  done
}

start=$(date +%s%N)
pids=
SECONDS=0
for w in $(seq 1 $W); do worker "$w" "$S/race.$w" & pids="$pids $!"; done
failed=0
for p in $pids; do wait "$p" || failed=1; done   # not a bare wait: the throwaway server is a background job of this shell too
end=$(date +%s%N)
if [ "$failed" -ne 0 ]; then
  cat "$S"/fail.* >&2
  echo "FAIL race.sh: a worker hit a backend error or its bound (backend: $BACKEND, ${SECONDS}s)"
  exit 1
fi

total=$(cat "$S"/race.[0-9]* | grep -c '^T')
uniq=$(cat "$S"/race.[0-9]* | awk '{print $1}' | sort -u | wc -l)
echo "--- statuses after race:"
$B ls --limit 100 | awk '{print $2}' | sort | uniq -c
echo "claims=$total unique=$uniq elapsed_ms=$(( (end-start)/1000000 )) (backend: $BACKEND)"
[ "$total" -eq 80 ] && [ "$uniq" -eq 80 ]
