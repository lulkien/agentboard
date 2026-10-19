#!/usr/bin/env bash
# Proves the long-poll: the waiter returns as soon as an event lands, and the
# elapsed time is ~the event latency, not the timeout.
set -u
cd "$(dirname "$0")/.."
DB=$PWD/.probe/wait.db
B=$PWD/target/release/agentboard
rm -f "$DB" "$DB-wal" "$DB-shm"
export AGENTBOARD_DB=$DB

AGENTBOARD_ACTOR=hermes $B add "wait test" --description "blocked work order" >/dev/null

( sleep 1.5; AGENTBOARD_ACTOR=human $B note T1 "go ahead, use nix" >/dev/null ) &

start=$(date +%s%N)
AGENTBOARD_ACTOR=claude1 $B wait T1 --timeout 15
echo "elapsed_ms=$(( ($(date +%s%N)-start)/1000000 ))"
wait
