#!/usr/bin/env bash
# Proves the long-poll: the waiter returns as soon as an event lands, and the
# elapsed time is ~the event latency, not the timeout. Runs on a throwaway board
# (tests/board.sh), never the default one; AGENTBOARD_TEST_BIN picks the binary.
set -u
cd "$(dirname "$0")/.."
B=${AGENTBOARD_TEST_BIN:-$PWD/target/release/agentboard}
S=$(mktemp -d /tmp/agentboard-wait.XXXXXX)
source tests/board.sh
trap 'board_stop; rm -rf "$S"' EXIT
use wait
export AGENTBOARD_DB=$BOARD
echo "backend: $BACKEND, board: $BOARD${AGENTBOARD_SURREAL_URL:+ at $AGENTBOARD_SURREAL_URL}"

AGENTBOARD_ACTOR=hermes $B add "wait test" --description "blocked work order" >/dev/null

( sleep 1.5; AGENTBOARD_ACTOR=human $B note T1 "go ahead, use nix" >/dev/null ) &
noter=$!

start=$(date +%s%N)
AGENTBOARD_ACTOR=claude1 $B wait T1 --timeout 15
rc=$?
echo "elapsed_ms=$(( ($(date +%s%N)-start)/1000000 )) (backend: $BACKEND)"
wait $noter   # not a bare wait: the throwaway server is a background job of this shell too
exit $rc
