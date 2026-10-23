#!/usr/bin/env bash
# Multi-process claim race: N processes, M tasks, every task claimed exactly once.
# This is the reason the store is multi-process; a single-process broker would pass
# a thread race and fail here.
set -u
B=${AGENTBOARD_TEST_BIN:-$PWD/target/debug/agentboard}
[ -x "$B" ] || { echo "FAIL: no binary at $B; cargo build first"; exit 1; }
WORKERS=${WORKERS:-8}
TASKS=${TASKS:-24}
S=$(mktemp -d "${TMPDIR:-/tmp}/agentboard-race.XXXXXX")
trap 'rm -rf "$S"' EXIT
export AGENTBOARD_DB="$S/board"
unset AGENTBOARD_MAP_SIZE_MB

AGENTBOARD_ACTOR=manager "$B" add "seed" >/dev/null
for i in $(seq 2 "$TASKS"); do
  AGENTBOARD_ACTOR=manager "$B" add "task $i" --priority "$i" >/dev/null
done

pids=()
for w in $(seq 1 "$WORKERS"); do
  (
    export AGENTBOARD_ACTOR="worker-$w"
    while :; do
      out=$("$B" claim 2>&1); code=$?
      if [ "$code" -eq 3 ]; then break; fi
      if [ "$code" -ne 0 ]; then echo "$out" > "$S/fail-$w"; exit 1; fi
      printf '%s\n' "$out" | head -1 | cut -d' ' -f1
    done
  ) > "$S/claims-$w" 2>&1 &
  pids+=($!)
done
failed=0
for pid in "${pids[@]}"; do wait "$pid" || failed=1; done
[ "$failed" -eq 0 ] || { echo "FAIL: a worker exited non-zero"; cat "$S"/fail-* 2>/dev/null; exit 1; }

cat "$S"/claims-* | rg -o '^T[0-9]+$' | sort > "$S/claimed"
total=$(wc -l < "$S/claimed")
unique=$(sort -u "$S/claimed" | wc -l)
if [ "$total" -ne "$TASKS" ]; then
  echo "FAIL: $total claims for $TASKS tasks"; exit 1
fi
if [ "$unique" -ne "$TASKS" ]; then
  echo "FAIL: $unique distinct tasks claimed, expected $TASKS (a task was claimed twice)"; exit 1
fi
claimed_events=$(AGENTBOARD_ACTOR=manager "$B" feed --kind claimed --limit 500 | rg -c ' claimed  T' )
if [ "$claimed_events" -ne "$TASKS" ]; then
  echo "FAIL: feed recorded $claimed_events claimed events, expected $TASKS"; exit 1
fi
echo "ok: $WORKERS processes claimed $TASKS tasks, $unique distinct, $claimed_events claimed events"
