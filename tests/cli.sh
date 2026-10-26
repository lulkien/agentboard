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

# --- claim rules and comment-only updates --------------------------------
fresh i
"$B" add "own todo" --assignee manager --new-actor >/dev/null
"$B" add "review row" --status review >/dev/null
"$B" add "blocked row" --status blocked --assignee manager >/dev/null
out=$("$B" claim 1)
has "claiming your assigned todo starts the work" "in_progress" "$out"
has "claiming your assigned todo bumps the revision" "revision 2" "$out"
out=$("$B" claim 1)
has "claiming a held in_progress row is idempotent" "revision 2" "$out"
out=$("$B" claim); code=$?
check "id-less claim skips review and blocked rows" 3 "$code"
out=$("$B" claim 2)
has "an explicit claim takes a review row" "review" "$out"
out=$("$B" claim 3 2>&1); code=$?
check "blocked is never claimable" 1 "$code"
has "blocked refusal names the status" "blocked cannot be claimed" "$out"
out=$("$B" update 3 --if-revision 1 --comment "a note without a field change")
has "comment-only update succeeds" "ok" "$out"
has "comment-only update leaves the revision" "revision 1" "$out"
has "comment-only update lands as a comment" "a note without a field change" "$("$B" feed --kind commented)"
out=$(AGENTBOARD_ACTOR=other "$B" update 3 --if-revision 1 --comment "still notes")
has "a foreign holder can still comment" "ok" "$out"
out=$("$B" update 3 --if-revision 1 2>&1); code=$?
check "no field and no comment exit" 1 "$code"
has "no field and no comment text" "task update has no field changes" "$out"
out=$("$B" feed --wait 1)
has "a no-cursor wait answers a page" "cursor " "$out"
check "a no-cursor wait returns no events" 0 "$(printf '%s\n' "$out" | grep -cE '^[0-9]+ ' || true)"
out=$(AGENTBOARD_ACTOR=bob "$B" update 3 --if-revision 1 --title "bob on a blocked row" 2>&1); code=$?
check "protected status still needs force" 1 "$code"
has "protected status refusal" "held by" "$out"

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

# --- derived epics and atomic close refusals -----------------------------
fresh epic
"$B" add "umbrella" --priority 20 >/dev/null
"$B" add "open child" --parent 1 --status in_progress --assignee manager >/dev/null
"$B" add "done child" --parent 1 --status done >/dev/null
"$B" add "dropped child" --parent 1 --status canceled >/dev/null
"$B" add "ordinary active" --status review --priority -1 >/dev/null
out=$(AGENTBOARD_ACTOR= "$B" epic 1); code=$?
check "epic needs no actor" 0 "$code"
check "epic has four lines" 4 "$(printf '%s\n' "$out" | wc -l | tr -d ' ')"
check "epic compact row" "$("$B" ls --all | rg '^T1 ')" "$(printf '%s\n' "$out" | head -1)"
check "epic status counts" "todo 0  in_progress 1  blocked 0  review 0  done 1  canceled 1" "$(printf '%s\n' "$out" | sed -n '2p')"
check "epic progress" "done 1/3" "$(printf '%s\n' "$out" | sed -n '3p')"
has "epic total and snapshot cursor" "total 3 cursor " "$out"
out=$("$B" epic 2 2>&1); code=$?
check "leaf epic exit" 1 "$code"
check "leaf epic text" "agentboard: task T2 is not an epic; an epic is a task with children" "$out"
out=$("$B" epic 999 2>&1); code=$?
check "missing epic exit" 1 "$code"
check "missing epic text" "agentboard: task T999 does not exist" "$out"
for status in done canceled; do
    out=$("$B" update 1 --if-revision 1 --status "$status" 2>&1); code=$?
    check "epic $status refusal exit" 1 "$code"
    check "epic $status refusal text" "agentboard: epic T1 has 1 open children; close or drop them first" "$out"
done
out=$("$B" brief)
check "brief epic first" T1 "$(printf '%s\n' "$out" | head -1 | cut -d' ' -f1)"
has "brief epic progress" "done 1/3" "$(printf '%s\n' "$out" | head -1)"
check "brief ordinary active second" T5 "$(printf '%s\n' "$out" | sed -n '2p' | cut -d' ' -f1)"
lacks "brief hides child rows" "T2 " "$out"
check "brief counts last" "todo 1  in_progress 1  blocked 0  review 1  done 1  canceled 1" "$(printf '%s\n' "$out" | tail -1)"
"$B" update 2 --if-revision 1 --status canceled >/dev/null
out=$("$B" update 1 --if-revision 1 --status done); code=$?
check "all terminal epic closes" 0 "$code"
has "closed epic record" "T1 done" "$out"

# --- terminal parents refuse new attachments, manager reopens and retries ---
for status in done canceled; do
    fresh "parent-$status"
    "$B" --db "$AGENTBOARD_DB" add parent --status "$status" >/dev/null
    "$B" --db "$AGENTBOARD_DB" add move >/dev/null
    before=$("$B" --db "$AGENTBOARD_DB" ls --all)
    for verb in add update; do
        if [ "$verb" = add ]; then
            out=$("$B" --db "$AGENTBOARD_DB" add new --parent 1 2>"$S/refusal"); code=$?
        else
            out=$("$B" --db "$AGENTBOARD_DB" update 2 --if-revision 1 --parent 1 2>"$S/refusal"); code=$?
        fi
        check "$status $verb exit" 1 "$code"
        check "$status $verb stdout" "" "$out"
        check "$status $verb exact refusal" "agentboard: parent T1 is $status; reopen it before adding children" "$(cat "$S/refusal")"
        check "$status $verb unchanged" "$before" "$("$B" --db "$AGENTBOARD_DB" ls --all)"
        has "$status parent revision unchanged" "revision 1" "$("$B" --db "$AGENTBOARD_DB" get 1)"
        has "$status child revision unchanged" "revision 1" "$("$B" --db "$AGENTBOARD_DB" get 2)"
    done
    "$B" --db "$AGENTBOARD_DB" update 1 --if-revision 1 --status todo >/dev/null
    out=$("$B" --db "$AGENTBOARD_DB" add new --parent 1); code=$?
    check "$status reopened add exit" 0 "$code"
    has "$status reopened add parent" "parent T1" "$out"
    out=$("$B" --db "$AGENTBOARD_DB" update 2 --if-revision 1 --parent 1); code=$?
    check "$status reopened reparent exit" 0 "$code"
    has "$status reopened reparent parent" "parent T1" "$out"
done

# --- a closed task refuses an owner change; reopening and assigning is one write ---
for status in done canceled; do
    fresh "closed-$status"
    "$B" add held --status "$status" --assignee alice --new-actor >/dev/null
    want="agentboard: $status task cannot change assignee; reopen it with an open status in the same update"
    before=$("$B" ls --all; "$B" get 1; "$B" feed --after 0)
    for args in "--assignee coder --new-actor" "--clear-assignee" "--assignee coder --new-actor --force --reason quota"; do
        # shellcheck disable=SC2086
        out=$("$B" update 1 --if-revision 1 $args 2>"$S/refusal"); code=$?
        check "$status owner change ($args) exit" 1 "$code"
        check "$status owner change ($args) stdout" "" "$out"
        check "$status owner change ($args) refusal" "$want" "$(cat "$S/refusal")"
        check "$status owner change ($args) unchanged" "$before" "$("$B" ls --all; "$B" get 1; "$B" feed --after 0)"
    done
    out=$("$B" update 1 --if-revision 1 --title renamed); code=$?
    check "$status closed edit exit" 0 "$code"
    out=$("$B" update 1 --if-revision 2 --status todo --assignee coder --new-actor); code=$?
    check "$status reopen and assign exit" 0 "$code"
    has "$status reopen and assign record" "T1 todo" "$out"
    out=$("$B" claim 1 --actor coder); code=$?
    check "$status claim after reopen exit" 0 "$code"
    has "$status claim after reopen status" "T1 in_progress" "$out"
done
fresh closing
"$B" add working --status in_progress --assignee alice --new-actor >/dev/null
out=$(AGENTBOARD_ACTOR=alice "$B" update 1 --if-revision 1 --status done --assignee coder --new-actor 2>&1); code=$?
check "close and hand over exit" 1 "$code"
has "close and hand over text" "done task cannot change assignee" "$out"

# --- a closed child reopens only after its closed parent, top-down ---------
for status in done canceled; do
    fresh "reopen-$status"
    "$B" add parent >/dev/null
    "$B" add child --parent 1 --status done >/dev/null
    "$B" update 1 --if-revision 1 --status "$status" >/dev/null
    before=$("$B" ls --all; "$B" get 1; "$B" get 2; "$B" feed --after 0)
    # Assign-only trips the owner rule first; reopen-and-assign trips the parent rule.
    out=$("$B" update 2 --if-revision 1 --assignee coder --new-actor 2>"$S/refusal"); code=$?
    check "$status closed pair assign-only exit" 1 "$code"
    check "$status closed pair assign-only refusal" "agentboard: done task cannot change assignee; reopen it with an open status in the same update" "$(cat "$S/refusal")"
    for flags in "" "--force --reason override"; do
        # shellcheck disable=SC2086
        out=$("$B" update 2 --if-revision 1 --status todo --assignee coder --new-actor --comment "must not land" $flags 2>"$S/refusal"); code=$?
        check "$status reopen under closed parent exit" 1 "$code"
        check "$status reopen under closed parent stdout" "" "$out"
        check "$status reopen under closed parent refusal" "agentboard: parent T1 is $status; reopen it before reopening T2" "$(cat "$S/refusal")"
        check "$status reopen under closed parent unchanged" "$before" "$("$B" ls --all; "$B" get 1; "$B" get 2; "$B" feed --after 0)"
    done
    out=$("$B" update 2 --if-revision 1 --status canceled); code=$?
    check "$status done to canceled under closed parent exit" 0 "$code"
    "$B" update 1 --if-revision 2 --status todo >/dev/null
    out=$("$B" update 2 --if-revision 2 --status todo --assignee coder --new-actor); code=$?
    check "$status parent first then child exit" 0 "$code"
    out=$("$B" claim 2 --actor coder); code=$?
    check "$status claim reopened child exit" 0 "$code"
    has "$status claim reopened child status" "T2 in_progress" "$out"
done

# --- wait: a typed verdict about the rows you name -------------------------
fresh wait
"$B" add "already done" --status done >/dev/null
"$B" add "moves during the wait" >/dev/null
out=$("$B" wait 1); code=$?
check "wait on a wanted row exit" 0 "$code"
has "wait answers matched" "matched: 1 of 1 watched in done,canceled" "$out"
has "wait prints the watched row" "T1 done" "$out"
has "wait labels an earlier event as last" "  last: 1 " "$out"
has "wait prints a cursor" "cursor " "$out"
check "wait ends with the attention line" "[ATTENTION] wait task is done, please check result" "$(printf '%s\n' "$out" | tail -1)"
check "wait orders the wanted statuses by the board" "matched: 1 of 1 watched in review,done" "$("$B" wait 1 --status done,review | head -1)"
check "wait is a read and appends nothing" 2 "$("$B" feed --after 0 | grep -c ' created  T')"
(
    sleep 0.5
    "$B" update 2 --if-revision 1 --status done >/dev/null
) &
writer=$!
out=$("$B" wait 2 --timeout 5); code=$?
wait "$writer"
check "wait on a row another process moves exit" 0 "$code"
has "wait sees the move" "matched: 1 of 1 watched in done,canceled" "$out"
has "wait shows the moved row's new status" "T2 done" "$out"
has "wait prints the event that moved it" "updated  T2" "$out"
lacks "wait does not claim causality for a moved row" "last:" "$out"

fresh wait-timeout
"$B" add "stays open" >/dev/null
"$B" add "already done" --status done >/dev/null
out=$("$B" wait 1 --timeout 1); code=$?
check "wait timeout exit" 3 "$code"
has "wait timeout text" "timeout: 0 of 1 watched in done,canceled after " "$out"
has "wait timeout still lists the row" "T1 todo" "$out"
has "wait timeout still prints a cursor" "cursor " "$out"
check "wait timeout ends with the timeout notice" "[ATTENTION] wait timed out, nothing matched; re-arm or check the board" "$(printf '%s\n' "$out" | tail -1)"
lacks "wait timeout has no matched notice" "[ATTENTION] wait task is done, please check result" "$out"
lacks "wait timeout has no last line" "last:" "$out"
has "wait timeout counts wanted rows" "timeout: 1 of 2 watched in done,canceled" "$("$B" wait 1 2 --timeout 1 | head -1)"

fresh wait-refusals
"$B" add "only row" >/dev/null
out=$("$B" wait 1 --status queued 2>&1); code=$?
check "wait unknown status exit" 1 "$code"
check "wait unknown status text" 'agentboard: unknown task status: "queued"' "$out"
out=$("$B" wait 999 2>&1); code=$?
check "wait missing task exit" 1 "$code"
check "wait missing task text" "agentboard: task T999 does not exist" "$out"
lacks "wait refusal prints no attention line" "[ATTENTION]" "$out"
out=$("$B" wait 1 --epic 2>&1); code=$?
check "wait epic on a leaf exit" 1 "$code"
check "wait epic on a leaf text" "agentboard: task T1 is not an epic; an epic is a task with children" "$out"
out=$("$B" wait 1 --status "" 2>&1); code=$?
check "wait empty status exit" 1 "$code"
has "wait empty status text" "wait: --status needs at least one of todo, in_progress, blocked, review, done, canceled" "$out"
out=$("$B" wait 2>&1); code=$?
check "wait without ids exit" 2 "$code"

fresh wait-epic
"$B" add "umbrella" >/dev/null
"$B" add "child one" --parent 1 --status in_progress --assignee manager >/dev/null
"$B" add "child two" --parent 1 --status in_progress --assignee manager >/dev/null
(
    sleep 0.5
    "$B" update 2 --if-revision 1 --status done >/dev/null
    sleep 0.5
    "$B" update 3 --if-revision 1 --status done >/dev/null
) &
writer=$!
out=$("$B" wait 1 --epic --timeout 5); code=$?
wait "$writer"
check "epic wait exit" 0 "$code"
has "epic wait watches the children" "matched: 2 of 2 watched in done,canceled" "$out"
has "epic wait shows the first child's move" "updated  T2" "$out"
has "epic wait needed the last child" "updated  T3" "$out"
lacks "epic wait hides the umbrella row itself" "T1 todo" "$out"

# --- wait --assignee: a handoff wakes the waiter with status unchanged ------
handoff_row() {
    fresh "$1"
    "$B" add "handoff row" --status in_progress --assignee analyst --new-actor >/dev/null
}
handoff_later() {
    (
        sleep 0.5
        AGENTBOARD_ACTOR=analyst "$B" update 1 --if-revision 1 --assignee coder --new-actor >/dev/null
    ) &
    writer=$!
}
handoff_row wait-assignee
handoff_later
out=$("$B" wait 1 --status review,done,blocked --assignee --timeout 5); code=$?
wait "$writer"
check "wait assignee on a handoff exit" 0 "$code"
check "wait assignee first line names the handoff" "matched: 0 of 1 watched in blocked,review,done; handoff T1 to coder" "$(printf '%s\n' "$out" | head -1)"
has "wait assignee leaves status alone" "T1 in_progress" "$out"
check "wait assignee handoff ends with the matched notice" "[ATTENTION] wait task is done, please check result" "$(printf '%s\n' "$out" | tail -1)"
has "wait assignee prints the update event" "updated  T1" "$out"

handoff_row wait-assignee-off
handoff_later
out=$("$B" wait 1 --status review,done,blocked --timeout 1); code=$?
wait "$writer"
check "wait without assignee ignores a handoff exit" 3 "$code"
has "wait without assignee times out on status" "timeout: 0 of 1 watched in blocked,review,done" "$out"

handoff_row wait-assignee-other
handoff_later
out=$("$B" wait 1 --status review,done,blocked --assignee reviewer --timeout 1); code=$?
wait "$writer"
check "wait assignee for another seat exit" 3 "$code"
has "wait assignee for another seat times out" "timeout: 0 of 1 watched" "$out"

handoff_row wait-assignee-named
handoff_later
out=$("$B" wait 1 --status review,done,blocked --assignee coder --timeout 5); code=$?
wait "$writer"
check "wait assignee naming the new owner exit" 0 "$code"
has "wait assignee naming the new owner text" "; handoff T1 to coder" "$out"

handoff_row wait-assignee-flags
out=$("$B" wait 1 --assignee --epic 2>&1); code=$?
check "wait assignee leaves --epic a flag exit" 1 "$code"
check "wait assignee leaves --epic a flag text" "agentboard: task T1 is not an epic; an epic is a task with children" "$out"
out=$("$B" wait 1 --assignee= 2>&1); code=$?
check "wait assignee empty name exit" 1 "$code"
has "wait assignee empty name text" "actor" "$out"

# --- release: one verb returns a dead holder's row to todo -----------------
fresh release
"$B" add "held by a dead seat" >/dev/null
"$B" update 1 --if-revision 1 --status in_progress --assignee coder --new-actor >/dev/null
out=$("$B" update 1 --if-revision 2 --release --force --reason "the coder seat hit its quota" \
      --comment "back in the pool for another seat"); code=$?
check "forced release exit" 0 "$code"
check "forced release starts with ok" "ok" "$(printf '%s\n' "$out" | head -1)"
check "forced release prints the todo record" \
    "T1 todo         prio  100 -          held by a dead seat" "$(printf '%s\n' "$out" | sed -n '2p')"
has "forced release bumps the revision" "revision 3" "$out"
has "forced release prints a cursor" "cursor " "$out"
has "forced release shows the comment in the feed" "back in the pool for another seat" "$("$B" feed --task 1 --kind updated)"
check "forced release leaves no assignee on the row" "T1 todo         prio  100 -          held by a dead seat" "$("$B" ls | rg '^T1 ')"

"$B" add "holder releases itself" >/dev/null
"$B" update 2 --if-revision 1 --status in_progress --assignee coder >/dev/null
out=$(AGENTBOARD_ACTOR=coder "$B" update 2 --if-revision 2 --release); code=$?
check "holder release exit" 0 "$code"
has "holder release returns the row" "T2 todo" "$out"
has "holder release bumps the revision" "revision 3" "$out"

out=$("$B" update 2 --if-revision 3 --release 2>&1); code=$?
check "holder-less release exit" 1 "$code"
check "holder-less release text" "agentboard: task T2 has no holder to release" "$out"

"$B" add "finished" --status done >/dev/null
out=$("$B" update 3 --if-revision 1 --release 2>&1); code=$?
check "done row release exit" 1 "$code"
check "done row release text" "agentboard: task T3 is done; nothing to release" "$out"

"$B" add "still held" >/dev/null
"$B" update 4 --if-revision 1 --status in_progress --assignee coder >/dev/null
out=$("$B" update 4 --if-revision 2 --release --status todo 2>&1); code=$?
check "--release with --status exit" 1 "$code"
has "--release with --status text" "choose --release or --status/--assignee/--clear-assignee, not both" "$out"
out=$("$B" update 4 --if-revision 2 --release --assignee bob 2>&1); code=$?
check "--release with --assignee exit" 1 "$code"
out=$("$B" update 4 --if-revision 2 --release --priority 5 2>&1); code=$?
check "--release with --priority exit" 1 "$code"
check "--release with --priority text" \
    "agentboard: choose --release or a field change, not both" "$out"
check "--release with --priority changes nothing" \
    "T4 in_progress  prio  100 coder      still held" "$("$B" ls | rg '^T4 ')"
out=$(AGENTBOARD_ACTOR=other "$B" update 4 --if-revision 2 --title "stolen" 2>&1); code=$?
check "foreign write on a live holder exit" 1 "$code"
has "held refusal names both recoverable paths" \
    "held by coder; --force with a reason takes it, or --release --force with a reason returns it to todo" "$out"
out=$("$B" update 4 --if-revision 2 --release --force 2>&1); code=$?
check "forced release without a reason exit" 1 "$code"
has "forced release without a reason text" "forced update requires a reason" "$out"
out=$("$B" update 4 --if-revision 2 --release --force --reason "the reviewer seat owns this now"); code=$?
check "forced release with a reason and no comment exit" 0 "$code"
has "forced release shows the reason in the feed" \
    "| the reviewer seat owns this now" "$("$B" feed --task 4 --kind updated)"

echo "$pass/$n passed (cli, lmdb)"
[ "$fail" -eq 0 ]
