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
for i in $(seq 1 80); do $B add "task $i" >/dev/null; done

worker() {
  local w=$1 out=$2
  : > "$out"
  while :; do
    local t
    t=$(AGENTBOARD_ACTOR="w$w" $B claim 2>&1 | head -1)
    if [ "$t" = "no ready tasks" ]; then break; fi
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
for w in $(seq 1 $W); do worker "$w" "$S/race.$w" & pids="$pids $!"; done
wait $pids   # not a bare wait: the throwaway server is a background job of this shell too
end=$(date +%s%N)

total=$(cat "$S"/race.[0-9]* | grep -c '^T')
uniq=$(cat "$S"/race.[0-9]* | awk '{print $1}' | sort -u | wc -l)
echo "--- statuses after race:"
$B ls --limit 100 | awk '{print $2}' | sort | uniq -c
echo "claims=$total unique=$uniq elapsed_ms=$(( (end-start)/1000000 )) (backend: $BACKEND)"
[ "$total" -eq 80 ] && [ "$uniq" -eq 80 ]
