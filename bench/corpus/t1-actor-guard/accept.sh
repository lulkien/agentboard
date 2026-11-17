#!/usr/bin/env bash
set -euo pipefail
bin=$1
tree=$2
db=$(mktemp -d /tmp/agentboard/scratch/bench-accept.XXXXXX)
export AGENTBOARD_DB="$db/board" AGENTBOARD_ACTOR=alice
"$bin" add seed >/dev/null
set +e
"$bin" --actor ' bad name' update 1 --if-revision 99 --comment hi >/dev/null 2>&1
status=$?
set -e
[[ $status -eq 1 ]] || { echo "invalid actor update exited $status, expected 1" >&2; exit 1; }
if "$bin" agents | grep -Fq 'bad name'; then
  echo "invalid actor was added to roster" >&2
  exit 1
fi
