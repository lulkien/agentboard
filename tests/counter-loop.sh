#!/usr/bin/env bash
# The counter-analyst protocol on a scratch board with the real binary: the row stays
# `planning` while the assignee alternates analyst <-> counter-analyst, the debate is
# comments, only the agreed plan reaches the analyst report, and the manager's analyst-stage
# wait fires on every handoff. No server, no live board.
set -u
B=${AGENTBOARD_TEST_BIN:-$PWD/target/debug/agentboard}
[ -x "$B" ] || { echo "FAIL: no binary at $B; cargo build first"; exit 1; }
S=$(mktemp -d "${TMPDIR:-/tmp}/agentboard-counter.XXXXXX")
trap 'rm -rf "$S"' EXIT
export AGENTBOARD_DB="$S/board" AGENTBOARD_PROJECT=0
unset AGENTBOARD_ACTOR AGENTBOARD_MAP_SIZE_MB
n=0 fail=0
ok() { n=$((n + 1)); }
bad() { n=$((n + 1)); fail=$((fail + 1)); echo "FAIL: $*"; }
has() { [[ "$3" == *"$2"* ]] && ok || bad "$1: missing '$2' in '$3'"; }
lacks() { [[ "$3" != *"$2"* ]] && ok || bad "$1: unexpected '$2' in '$3'"; }
rev() { "$B" --actor manager get "$1" | sed -n 's/^revision //p'; }
field() { "$B" --actor manager get "$1" | head -1; }
cursor=0
say() { # say <actor> <id> <comment>: one comment; remembers the cursor the next handoff is waited after
  local out
  out=$("$B" --actor "$1" comment "$2" --body "$3") || bad "comment by $1 failed: $out"
  cursor=$(sed -n 's/^cursor //p' <<<"$out" | tail -1)
}
hand() { # hand <actor> <id> <to> [update args]: assign the row and check the manager's wait names the seat
  local actor=$1 id=$2 to=$3 out w
  shift 3
  out=$("$B" --actor "$actor" update "$id" --if-revision "$(rev "$id")" --assignee "$to" --new-actor "$@" 2>&1) || bad "$actor -> $to update failed: $out"
  w=$("$B" --actor manager wait "$id" --status reviewing,done,blocked --assignee --after "$cursor" --timeout 10 2>&1 | head -1)
  has "wait names the handoff $actor -> $to" "handoff T$id to $to" "$w"
}
row() { "$B" --actor manager add "$1" --project 0 --status planning --assignee analyst --new-actor --goal "g" --description "d" | sed -n 's/^T\([0-9]*\) .*/\1/p' | head -1; }

# --- agreed in round 2: the plan reaches the report only after the pass -------------
id=$(row "agreed row")
has "manager files the row planning/analyst" "planning" "$(field "$id")"
say analyst "$id" "analyst 1.1 draft: Files, Functions, Behaviour, Must not change, Proof"
hand analyst "$id" counter-analyst
has "row stays planning while counter-analyst holds it" "planning" "$(field "$id")"
has "row is held by counter-analyst" "counter-analyst" "$(field "$id")"
say counter-analyst "$id" "counter-analyst 1.1: objections
1. Blocking: the draft misses the caller in src/x.rs (evidence: rg output)
2. Note: name the test"
hand counter-analyst "$id" analyst
say analyst "$id" "analyst 1.2: 1 accepted, the plan now names src/x.rs"
hand analyst "$id" counter-analyst
say counter-analyst "$id" "counter-analyst 1.2: pass"
hand counter-analyst "$id" analyst
full=$("$B" --actor manager get "$id" --full)
lacks "no report section while the debate runs" "## Analyst plan" "$full"
printf '## Analyst plan 1\nTurn: analyst 1\nFiles: src/x.rs\n' >"$S/plan"
out=$("$B" --actor analyst update "$id" --if-revision "$(rev "$id")" --analyst-report-file "$S/plan" 2>&1) || bad "analyst report write failed: $out"
say analyst "$id" "analyst 1: final plan written to the report"
hand analyst "$id" coder --status implementing
full=$("$B" --actor manager get "$id" --full)
has "agreed plan is in the analyst report" "## Analyst plan 1" "$full"
has "row now implementing for the coder" "implementing" "$(field "$id")"
feed=$("$B" --actor manager feed --task "$id" --after 0 --full)
has "the debate is row comments" "counter-analyst 1.1: objections" "$feed"
lacks "no Counter-analyst report section" "## Counter-analyst" "$full"

# --- the cap: five blocking reviews end blocked, held by counter-analyst ------------
id=$(row "capped row")
for r in 1 2 3 4 5; do
  say analyst "$id" "analyst 1.$r draft"
  hand analyst "$id" counter-analyst
  if [ "$r" -lt 5 ]; then
    say counter-analyst "$id" "counter-analyst 1.$r: objections
1. Blocking: still wrong (evidence $r)"
    hand counter-analyst "$id" analyst
  fi
done
say counter-analyst "$id" "counter-analyst 1.5: objections
1. Blocking: still wrong (evidence 5)
cap: 5 reviews reached; disagreement: the goal is unfalsifiable"
out=$("$B" --actor counter-analyst update "$id" --if-revision "$(rev "$id")" --status blocked 2>&1) || bad "cap block failed: $out"
line=$(field "$id")
has "capped row is blocked" "blocked" "$line"
has "capped row is still held by counter-analyst" "counter-analyst" "$line"
w=$("$B" --actor manager wait "$id" --status blocked --timeout 5 2>&1 | head -1)
has "the manager's wait sees the blocked row" "matched" "$w"
feed=$("$B" --actor manager feed --task "$id" --after 0 --full)
has "the cap comment names the disagreement" "cap: 5 reviews reached" "$feed"

# --- the human accepts the capped draft: back through the analyst, report before coder -----
out=$("$B" --actor manager update "$id" --if-revision "$(rev "$id")" --status planning --assignee analyst --force --reason "human decision: accept the draft as it stands" 2>&1) || bad "manager return failed: $out"
cursor=$(sed -n 's/^cursor //p' <<<"$out" | tail -1)
say manager "$id" "human decision: accept analyst 1.5 draft as it stands"
has "the capped row is back with the analyst, still planning" "planning" "$(field "$id")"
full=$("$B" --actor manager get "$id" --full)
lacks "capped row has no report before the analyst writes it" "## Analyst plan" "$full"
printf '## Analyst plan 1\nTurn: analyst 1\nFiles: accepted draft\n' >"$S/plan2"
out=$("$B" --actor analyst update "$id" --if-revision "$(rev "$id")" --analyst-report-file "$S/plan2" 2>&1) || bad "analyst report after accept failed: $out"
say analyst "$id" "analyst 1: accepted draft written to the report"
hand analyst "$id" coder --status implementing
full=$("$B" --actor manager get "$id" --full)
has "the coder finds the plan in the report" "## Analyst plan 1" "$full"

echo "$((n - fail))/$n passed (counter loop)"
[ "$fail" -eq 0 ]
