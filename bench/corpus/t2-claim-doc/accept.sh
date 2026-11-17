#!/usr/bin/env bash
set -euo pipefail
bin=$1
tree=$2
! grep -Fq 'not claimable' "$tree/PROTOCOL.md"
grep -Fq 'cannot be claimed' "$tree/PROTOCOL.md"
files=$(git -C "$tree" diff --name-only HEAD^ HEAD)
[[ "$files" == "PROTOCOL.md" ]] || { echo "changed paths: $files" >&2; exit 1; }
