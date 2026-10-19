#!/usr/bin/env bash
# Concurrency probe: N workers drain a queue of tasks. Proves no task is
# claimed twice and none is lost.
set -u
B=$PWD/target/release/agentboard
export AGENTBOARD_DB=$PWD/.probe/race.db
rm -f "$AGENTBOARD_DB" "$AGENTBOARD_DB"-*
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
for w in $(seq 1 $W); do worker "$w" ".probe/race.$w" & done
wait
end=$(date +%s%N)

total=$(cat .probe/race.[0-9]* | grep -c '^T')
uniq=$(cat .probe/race.[0-9]* | awk '{print $1}' | sort -u | wc -l)
echo "claims=$total unique=$uniq elapsed_ms=$(( (end-start)/1000000 ))"
echo "--- statuses after race:"
$B ls --limit 100 | awk '{print $2}' | sort | uniq -c
