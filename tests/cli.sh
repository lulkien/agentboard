#!/usr/bin/env bash
# CLI surface for the LMDB board. Every section uses a scratch board: no server,
# no live state, no shared state between sections.
set -u
B=${AGENTBOARD_TEST_BIN:-$PWD/target/debug/agentboard}
[ -x "$B" ] || { echo "FAIL: no binary at $B; cargo build first"; exit 1; }
S=$(mktemp -d "${TMPDIR:-/tmp}/agentboard-cli.XXXXXX")
trap 'rm -rf "$S"' EXIT
export AGENTBOARD_ACTOR=manager
unset AGENTBOARD_MAP_SIZE_MB
n=0
pass=0 fail=0
ok() { n=$((n + 1)); pass=$((pass + 1)); }
bad() { n=$((n + 1)); fail=$((fail + 1)); echo "FAIL: $*"; }
check() { local name=$1 want=$2 got=$3; [[ "$got" == "$want" ]] && ok || bad "$name: want '$want' got '$got'"; }
has() { local name=$1 needle=$2 hay=$3; [[ "$hay" == *"$needle"* ]] && ok || bad "$name: missing '$needle' in '$hay'"; }
lacks() { local name=$1 needle=$2 hay=$3; [[ "$hay" != *"$needle"* ]] && ok || bad "$name: unexpected '$needle' in '$hay'"; }
fresh() { AGENTBOARD_DB="$S/board-$1"; export AGENTBOARD_DB; }

# --- path and add ----------------------------------------------------------
fresh a
out=$("$B" path) || bad "path exit"
check "path prints the resolved board" "$AGENTBOARD_DB" "$out"
out=$("$B" add "first task" --description "context here" --goal "done when green" --priority 5)
has "add prints ok" "ok" "$out"
has "add prints the record" "T1 todo" "$out"
has "add prints a cursor" "cursor 1" "$out"
"$B" add "second task" --priority 1 >/dev/null
out=$("$B" ls)
has "ls totals" "total 2" "$out"
check "lowest priority first" "T2" "$(printf '%s\n' "$out" | head -1 | cut -d' ' -f1)"
has "ls prints a cursor" "cursor " "$out"
out=$("$B" ls --status todo --limit 1)
has "status filter keeps the row" "T2" "$out"
check "limit one row" 1 "$(printf '%s\n' "$out" | rg -c '^T[0-9]+ ')"
out=$("$B" ls --status done)
check "explicit done filter lists nothing" "no tasks" "$(printf '%s\n' "$out" | head -1)"

# --- values that start with a dash are values -----------------------------
fresh b
out=$("$B" add "- dash title" --description "- dash body") || bad "hyphen title exit"
has "hyphen title stored" "- dash title" "$("$B" ls)"
has "hyphen description stored" "- dash body" "$("$B" get 1)"

# --- refusals name what is wrong ------------------------------------------
fresh c
out=$(AGENTBOARD_ACTOR= "$B" add "no actor" 2>&1); code=$?
check "actor refusal exit" 1 "$code"
has "actor refusal names the fix" "AGENTBOARD_ACTOR" "$out"
out=$("$B" add "bad status" --status queued 2>&1); code=$?
check "unknown status exit" 1 "$code"
has "unknown status names the value" 'unknown task status: "queued"' "$out"
out=$("$B" add "unknown assignee" --assignee nobody 2>&1); code=$?
check "unknown assignee exit" 1 "$code"
has "unknown assignee text" "unknown assignee" "$out"
out=$("$B" add "stranger" --assignee nobody --new-actor)
has "new actor registers the name" "nobody" "$out"

# --- truncation, full text, comments -------------------------------------
fresh d
long=$(printf 'x%.0s' {1..400})
"$B" add "long body" --description "$long" --goal "short goal" >/dev/null
out=$("$B" get 1)
has "compact description is truncated" "…" "$out"
lacks "compact description hides the tail" "$long" "$out"
out=$("$B" get 1 --full)
has "full description is complete" "$long" "$out"
out=$("$B" comment 1 --body "starting now")
has "comment ack" "cursor 2" "$out"
out=$("$B" get 1)
has "a comment leaves the revision" "revision 1" "$out"
out=$("$B" comment 1 --body "-dash body")
has "hyphen comment body accepted" "ok" "$out"

# --- long comment text is reachable, and a long title survives --full -----
fresh d2
long_comment=$(printf 'c%.0s' {1..400})
long_title=$(printf 't%.0s' {1..100})
"$B" add "$long_title" >/dev/null
"$B" comment 1 --body "$long_comment" >/dev/null
out=$("$B" feed --kind commented)
lacks "compact feed truncates the comment" "$long_comment" "$out"
has "compact feed marks the cut" "…" "$out"
out=$("$B" feed --kind commented --full)
has "feed --full returns the whole comment" "$long_comment" "$out"
out=$("$B" get 1)
has "compact record truncates the title" "…" "$out"
out=$("$B" get 1 --full)
has "get --full returns the whole title" "$long_title" "$out"
lacks "get --full leaves no marker" "…" "$out"
indented=$'line one\n    line two'
"$B" add "indented body" --description "$indented" >/dev/null
out=$("$B" get 2 --full)
has "full text keeps inner whitespace" "line two" "$out"
check "full text keeps the indented line intact" 1 "$(printf '%s\n' "$out" | rg -c '^    line two$')"

# --- a negative priority is a value, and history is reachable -------------
fresh d3
"$B" add "urgent" --priority -3 >/dev/null
has "negative priority stored" "prio   -3" "$("$B" ls)"
"$B" add "later" --priority 9 >/dev/null
out=$("$B" feed --after 0)
first=$(printf '%s\n' "$out" | head -1)
has "after 0 starts at the oldest event" "created" "$first"
check "after 0 replays both creations" 2 "$(printf '%s\n' "$out" | rg -c ' created  T')"
out=$("$B" update 1 --if-revision 1 --assignee bob --clear-assignee 2>&1); code=$?
check "assignee and clear-assignee conflict exit" 1 "$code"
has "assignee and clear-assignee conflict text" "not both" "$out"

# --- revision guard and a forced take-over -------------------------------
fresh e
"$B" add "guarded" >/dev/null
out=$("$B" update 1 --if-revision 1 --status in_progress --assignee alice --new-actor)
has "update moves status" "in_progress" "$out"
out=$("$B" update 1 --if-revision 1 --title "stale" 2>&1); code=$?
check "stale revision exit" 1 "$code"
has "stale revision names both numbers" "stale task revision 1" "$out"
out=$("$B" update 1 --if-revision 2 --title "stolen" 2>&1); code=$?
check "foreign holder exit" 1 "$code"
has "foreign holder names the holder" "held by alice" "$out"
out=$("$B" update 1 --if-revision 2 --title "forced" --force 2>&1); code=$?
check "force without reason exit" 1 "$code"
has "force without reason text" "reason" "$out"
out=$("$B" update 1 --if-revision 2 --title "forced" --force --reason quota)
has "forced update bumps the revision" "revision 3" "$out"

# --- claim: lowest priority, then no work, then a held task ---------------
fresh f
"$B" add "low" --priority 50 >/dev/null
"$B" add "high" --priority 10 >/dev/null
out=$("$B" claim)
has "claim takes the highest priority" "high" "$out"
has "claim moves it to in_progress" "in_progress" "$out"
out=$("$B" claim)
has "second claim takes the next one" "low" "$out"
out=$("$B" claim); code=$?
check "no work exit" 3 "$code"
check "no work text" "no ready tasks" "$out"
out=$(AGENTBOARD_ACTOR=other "$B" claim 1 2>&1); code=$?
check "claim of a held task exit" 1 "$code"
has "claim refusal names the holder" "held by manager" "$out"

# --- feed: filters, cursors, foreign boards ------------------------------
fresh g
"$B" add "feed task" >/dev/null
"$B" update 1 --if-revision 1 --assignee alice --new-actor >/dev/null
out=$("$B" feed --kind created)
has "kind filter finds the creation" "created" "$out"
out=$("$B" feed --recipient alice)
has "recipient filter finds the handoff" "-> alice" "$out"
cursor=$(printf '%s\n' "$out" | rg '^cursor ' | sed 's/^cursor //')
out=$("$B" feed --after "$cursor")
check "resumed feed has no events" "" "$(printf '%s\n' "$out" | rg '^[0-9]+ ' || true)"
out=$("$B" feed --after "other-board:1" 2>&1); code=$?
check "foreign cursor exit" 1 "$code"
has "foreign cursor text" "different board" "$out"
out=$("$B" feed --limit 1 --kind created --after "0:0" 2>&1); code=$?
check "cursor from another instance exit" 1 "$code"

# --- brief, agents, reset ------------------------------------------------
fresh h
"$B" add "brief task" >/dev/null
out=$("$B" brief)
has "brief counts todo" "todo 1" "$out"
out=$("$B" agents)
has "agents lists the manager" "manager" "$out"
out=$("$B" reset 2>&1); code=$?
check "reset without yes exit" 1 "$code"
has "reset without yes text" "pass --yes" "$out"
out=$("$B" reset --yes)
has "reset succeeded" "new board instance" "$out"
out=$("$B" ls)
has "reset leaves an empty board" "total 0" "$out"

echo "$pass/$n passed (cli, lmdb)"
[ "$fail" -eq 0 ]
