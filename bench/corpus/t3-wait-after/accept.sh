#!/usr/bin/env bash
set -euo pipefail
bin=$1
tree=$2
db=$(mktemp -d /tmp/agentboard/scratch/bench-accept.XXXXXX)
export AGENTBOARD_DB="$db/board" AGENTBOARD_ACTOR=alice
"$bin" add 'handoff task' >/dev/null
"$bin" wait 1 --timeout 1 >"$db/first.out" || [[ $? -eq 3 ]]
cursor=$(sed -n 's/^cursor //p' "$db/first.out" | tail -n 1)
[[ -n "$cursor" ]] || { echo "first wait did not print a cursor" >&2; exit 1; }
"$bin" update 1 --if-revision 1 --assignee coder --new-actor --comment handoff >/dev/null
"$bin" wait 1 --after "$cursor" --assignee --timeout 1 >"$db/second.out"
grep -Fq 'handoff T1 to coder' "$db/second.out"
set +e
"$bin" wait 1 --assignee --timeout 1 >"$db/without-after.out" 2>&1
status=$?
set -e
[[ $status -eq 3 ]] || { echo "wait without --after exited $status, expected 3" >&2; exit 1; }
