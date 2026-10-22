#!/usr/bin/env bash
# CLI contract: exit codes, stderr text, flag spellings, no ANSI. These are
# the checks mcp_client.py cannot make (it reads tool text, not stderr/exit).
set -u
cd "$(dirname "$0")/.."
B=${AGENTBOARD_TEST_BIN:-$PWD/target/release/agentboard}
S=$(mktemp -d /tmp/agentboard-cli.XXXXXX)
source tests/board.sh
trap 'board_stop; rm -rf "$S"' EXIT
# Every command names its DB; this default only catches one that forgets.
use stray
export AGENTBOARD_DB=$BOARD AGENTBOARD_ACTOR=cli
unset NO_COLOR
pass=0 total=0
rc=0 out="" err=""

run() { out=$("$@" 2>"$S/err"); rc=$?; err=$(<"$S/err"); }
check() { # check <name> <condition-result: 0 = ok>
  total=$((total + 1))
  if [ "$2" -eq 0 ]; then pass=$((pass + 1)); echo "PASS $1"; else echo "FAIL $1 :: rc=$rc out=[$out] err=[$err]"; fi
}
has() { [[ "$1" == *"$2"* ]]; }

use a; D=$BOARD
# --flag=value equivalence for --db, --actor, --description
run $B --db="$D" add one --description="d=1" --assignee=zed;           check "--db=PATH accepted" $rc
run $B --db "$D" add two --description "d=2";                          check "--db PATH accepted" $rc
run $B add three --db "$D";                                            check "--db after the subcommand" $rc
run $B --db "$D" ls;                                                   check "all spellings hit one db" $([ "$(grep -c . <<<"$out")" = 3 ] && echo 0 || echo 1)
run $B --db "$D" get T1;                                               check "--description=V stored" $(has "$out" "d=1" && echo 0 || echo 1)
run $B --actor=zed --db "$D" note T1 "by zed";                         check "--actor=NAME" $rc
run $B --db "$D" get T1;                                               check "actor recorded" $(has "$out" "zed note" && echo 0 || echo 1)
run $B note T1 --actor me "between" --db "$D";                         check "note T1 --actor me text --db X" $rc
run $B --db "$D" get T1;                                               check "actor between positionals recorded" $(has "$out" "me note" && echo 0 || echo 1)
run $B --db "$D" add four --spec "via spec";                           check "--spec accepted" $rc
run $B --db "$D" get T4;                                               check "--spec is --description" $(has "$out" "via spec" && echo 0 || echo 1)
run $B --db "$D" claim T1; run $B --db "$D" claim T2
run $B --db "$D" claim T1;                                             check "claim of a held task exits 1 with the refusal text" $([ $rc -eq 1 ] && has "$err" "agentboard: T1 working by" && has "$err" "manager: agentboard status T1 ready" && echo 0 || echo 1)
use empty; run $B --db "$BOARD" claim; use a;                                       check "id-less claim on an empty board: exit 0, no ready tasks" $([ $rc -eq 0 ] && has "$out" "no ready tasks" && echo 0 || echo 1)
run $B wait T1 T2 --db "$D" --status working;                          check "wait T1 T2 --db X after positionals: match exit 0" $rc
run $B --db "$D" wait T1 T2 --status done --timeout 1;                 check "wait timeout exits 3" $([ $rc -eq 3 ] && echo 0 || echo 1)
check "timeout first line" $(has "$out" "timeout:" && echo 0 || echo 1)

# misuse is exit 1, never clap's 2, with the agentboard: prefix
run $B --asignee claude add x --db "$D"
check "unknown flag exits 1" $([ $rc -eq 1 ] && has "$err" "agentboard: " && has "$err" "'--asignee'" && echo 0 || echo 1)
run $B --db "$D" add y --asignee claude
check "unknown flag carries the tip" $([ $rc -eq 1 ] && has "$err" "tip:" && has "$err" "--assignee" && echo 0 || echo 1)
run $B --db "$D" ls;                                                   check "failed adds created nothing" $(has "$out" " x" || has "$out" " y" && echo 1 || echo 0)
run $B --dat /tmp ls;                                                  check "typo'd --db exits 1" $([ $rc -eq 1 ] && echo 0 || echo 1)
run $B --db "$D" add z --priority abc;                                 check "--priority abc exits 1" $([ $rc -eq 1 ] && echo 0 || echo 1)
run $B --db "$D" ls --limit abc;                                       check "--limit abc exits 1" $([ $rc -eq 1 ] && echo 0 || echo 1)
run $B --db "$D" wait T1 --after abc;                                  check "--after abc exits 1" $([ $rc -eq 1 ] && echo 0 || echo 1)
run $B --db "$D" wait T1 --status rview;                               check "bad status stays a store error, exit 1" $([ $rc -eq 1 ] && has "$err" "bad status" && echo 0 || echo 1)

# help and version: stdout, exit 0, no ESC byte
for args in "--help" "--version" "add --help" "wait --help"; do
  run $B $args
  esc=$(printf '%s' "$out$err" | LC_ALL=C grep -c $'\033' || true)
  check "$args: exit 0, stdout, no ESC" $([ $rc -eq 0 ] && [ -n "$out" ] && [ -z "$err" ] && [ "$esc" = 0 ] && echo 0 || echo 1)
done
run $B --version;                                                      check "--version is the crate version" $(has "$out" "0.1.0" && echo 0 || echo 1)
run $B --help;                                                         check "help states the exit codes and hyphen rule" $(has "$out" "3 wait timed out" && has "$out" "taken literally" && echo 0 || echo 1)

# leading '-' values are literal, in both spellings
run $B --db "$D" note T1 "- bullet";                                   check "note T1 \"- bullet\"" $rc
run $B --db "$D" add five --description "- bullet" --goal "- g";       check "--description \"- bullet\"" $rc
run $B --db "$D" get T5 --full;                                        check "dash description stored" $(has "$out" $'description:\n- bullet' && has "$out" $'goal:\n- g' && echo 0 || echo 1)
run $B --db "$D" add six --priority -5;                                check "--priority -5" $rc
run $B --db "$D" get T6;                                               check "negative priority stored" $(has "$out" "priority -5" && echo 0 || echo 1)
run $B --db "$D" add "- dash title";                                   check "title starting with '-'" $rc
run $B --db "$D" note T1 --bogus;                                      check "note T1 --bogus is literal text (documented)" $rc
run $B --db "$D" get T1 --full;                                        check "--bogus stored" $(has "$out" "--bogus" && echo 0 || echo 1)
run $B --db "$D" note T1 -- "- after dashdash";                        check "-- separator" $rc

# session registry verbs parse (liveness itself is stub-driven in dispatch.sh)
run $B --db "$D" register tmp-row;                                     check "register" $rc
run $B --db "$D" unregister tmp-row;                                   check "unregister removes the row, exit 0" $([ $rc -eq 0 ] && has "$out" "removed" && echo 0 || echo 1)
run $B --db "$D" unregister tmp-row;                                   check "unregister of an unknown name exits 1" $([ $rc -eq 1 ] && has "$err" "no registered session" && echo 0 || echo 1)
run $B sessions prune --bogus;                                         check "sessions prune rejects unknown flags, exit 1" $([ $rc -eq 1 ] && has "$err" "'--bogus'" && echo 0 || echo 1)
run $B sessions prune --help;                                          check "sessions prune --help documents --dry-run" $([ $rc -eq 0 ] && has "$out" "--dry-run" && echo 0 || echo 1)

# --- tail and send: stub-driven (herdr + tmux) -----------------------------------
# stub herdr and tmux binaries that we can control
H=$S/hbin
mkdir -p "$H"
cat >"$H/herdr" <<'HERDR'
#!/bin/sh
echo "herdr $*" >>"$LOG"
if [ "$1 $2" = "agent list" ]; then
  echo '{"result":{"agents":[{"agent":"claude","name":"coder","pane_id":"wB:p1"},{"agent":"codex","name":"reviewer","pane_id":"wB:p2"}]}}'
  exit 0
fi
if [ "$1 $2 $3" = "pane read wB:p1" ]; then
  echo -e "line 1\nline 2\nline 3"
  exit 0
fi
if [ "$1 $2 $3" = "pane send-text wB:p2" ] && [ "$4" = "hello world" ]; then
  exit 0
fi
if [ "$1 $2 $3" = "pane send-keys wB:p2" ] && [ "$4" = "enter" ]; then
  exit 0
fi
echo "no such pane" >&2
exit 1
HERDR
cat >"$H/tmux" <<'TMUX'
#!/bin/sh
echo "tmux $*" >>"$LOG"
if [ "$1" = "has-session" ]; then
  exit 0
fi
if [ "$1" = "capture-pane" ]; then
  echo -e "tmux line 1\ntmux line 2"
  exit 0
fi
if [ "$1" = "send-keys" ]; then
  exit 0
fi
exit 1
TMUX
chmod +x "$H/herdr" "$H/tmux"
export PATH=$H:$PATH
export LOG=$S/calls

run $B --db "$D" register coder --agent claude;                        check "register coder for herdr tail" $rc
run $B --db "$D" register reviewer --agent codex;                      check "register reviewer for herdr send" $rc

# tail via herdr stub
: >"$LOG"
run $B --db "$D" tail coder --lines 2
check "tail via herdr returns last N lines" $([ $rc -eq 0 ] && has "$out" "line 2" && has "$out" "line 3" && ! has "$out" "line 1" && echo 0 || echo 1)
check "tail via herdr calls herdr pane read with pane_id and --lines" $(grep -q "herdr pane read wB:p1 --lines 2" "$LOG" && echo 0 || echo 1)

# tail falls back to tmux stub when herdr agent not found
: >"$LOG"
run $B --db "$D" tail unknown --lines 1
check "tail falls back to tmux when herdr agent missing" $([ $rc -eq 0 ] && has "$out" "tmux line" && echo 0 || echo 1)
check "tail fallback calls tmux capture-pane" $(grep -q "tmux capture-pane -p -t unknown" "$LOG" && echo 0 || echo 1)

# send via herdr stub
: >"$LOG"
run $B --db "$D" send reviewer "hello world"
check "send via herdr issues send-text and send-keys enter" $([ $rc -eq 0 ] && has "$out" "ok sent" && echo 0 || echo 1)
check "send via herdr calls send-text then send-keys enter" $(grep -q "herdr pane send-text wB:p2 hello world" "$LOG" && grep -q "herdr pane send-keys wB:p2 enter" "$LOG" && echo 0 || echo 1)

# both-fail error names herdr and tmux
: >"$LOG"
cat >"$H/herdr" <<'HERDR'
#!/bin/sh
echo "herdr $*" >>"$LOG"
if [ "$1 $2" = "agent list" ]; then
  echo "herdr failed" >&2
  exit 1
fi
exit 1
HERDR
cat >"$H/tmux" <<'TMUX'
#!/bin/sh
echo "tmux $*" >>"$LOG"
echo "no server running" >&2
exit 1
TMUX
chmod +x "$H/herdr" "$H/tmux"
run $B --db "$D" tail nosuch --lines 1
check "both-fail error names herdr and tmux" $([ $rc -eq 1 ] && has "$err" "herdr: not a herdr agent with pane_id" && has "$err" "tmux: no server running" && echo 0 || echo 1)

# restore normal PATH
export PATH=$(echo $PATH | sed "s|$H:||")
unset LOG

# --- epics: own table, own ids, project path -----------------------------------
use e; E=$BOARD
mkdir -p "$S/proj"
run $B --db "$E" epic add "Ship it" --path "$S/proj/" --goal "g"
check "epic add with a path: E1 open, trailing slash stripped, existing dir => no warning" $([ $rc -eq 0 ] && [ "$out" = "E1 open $S/proj" ] && echo 0 || echo 1)
run $B --db "$E" epic add "Later" --path "$S/not/there/yet"
check "epic add with a missing path warns once and still creates it" $([ $rc -eq 0 ] && has "$out" "E2 open $S/not/there/yet" && [ "$(grep -c '^warning:' <<<"$out")" = 1 ] && echo 0 || echo 1)
run bash -c "cd '$S' && export HOME=/home/xx && '$B' --db '$E' epic add Rel --path ./proj/../proj/sub && '$B' --db '$E' epic add Home --path '~/work/'"
check "relative path resolves against the cwd, ~ against HOME, both stored absolute" $(has "$out" "E3 open $S/proj/sub" && has "$out" "E4 open /home/xx/work" && echo 0 || echo 1)
run $B --db "$E" epic add "No path"
check "epic without a path is fine" $([ "$out" = "E5 open" ] && echo 0 || echo 1)
run $B --db "$E" epic ls
check "epic ls: open epics with counts, path and the does-not-exist mark" $(has "$out" "E1 open 0/0 done $S/proj Ship it" && has "$out" "$S/not/there/yet (does not exist) Later" && echo 0 || echo 1)
run $B --db "$E" epic get E2
check "epic get marks a missing path" $(has "$out" "path: $S/not/there/yet (does not exist)" && echo 0 || echo 1)
run $B --db "$E" epic status E5 dropped "not needed"
check "epic status E5 dropped" $([ $rc -eq 0 ] && has "$out" "E5 dropped #" && echo 0 || echo 1)
run $B --db "$E" epic ls --status dropped;                             check "epic ls --status dropped" $(has "$out" "E5 dropped" && echo 0 || echo 1)
run $B --db "$E" epic status E5 finished;                              check "epic status rejects an unknown status, exit 1" $([ $rc -eq 1 ] && has "$err" "bad epic status" && echo 0 || echo 1)
run $B --db "$E" epic get E99;                                         check "epic get of an unknown epic exits 1" $([ $rc -eq 1 ] && has "$err" "no such epic E99" && echo 0 || echo 1)

# tasks point at an epic and/or another task
run $B --db "$E" add impl --epic E1;                                   check "add --epic E1" $([ $rc -eq 0 ] && echo 0 || echo 1)
run $B --db "$E" add review --epic E1
run $B --db "$E" add "fix it" --parent T2
check "subtask --parent T2 inherits the epic" $([ $rc -eq 0 ] && $B --db "$E" get T3 | grep -q "^epic: E1 Ship it" && $B --db "$E" get T3 | grep -q "^parent: T2" && echo 0 || echo 1)
run $B --db "$E" children E1;                                          check "children E1 = every task in the epic" $(has "$out" "T1 " && has "$out" "T2 " && has "$out" "T3 " && echo 0 || echo 1)
run $B --db "$E" children T2;                                          check "children T2 = its subtasks only" $(has "$out" "T3 " && ! has "$out" "T1 " && echo 0 || echo 1)
run $B --db "$E" wait --epic E1 --status ready --timeout 1;            check "wait --epic E1 watches the epic's tasks" $([ $rc -eq 0 ] && has "$out" "matched: 3 of 3 watched" && echo 0 || echo 1)

# the invariants and the two id spaces
run $B --db "$E" add x --epic E2 --parent T2
check "explicit epic that disagrees with the parent's: exit 1" $([ $rc -eq 1 ] && has "$err" "parent T2 belongs to E1, not E2" && echo 0 || echo 1)
run $B --db "$E" parent T2 T3;                                         check "re-parenting into your own subtree is a cycle: exit 1" $([ $rc -eq 1 ] && has "$err" "cycle: T2 is already an ancestor of T3" && echo 0 || echo 1)
run $B --db "$E" parent T2 T2;                                         check "self-parent: exit 1" $([ $rc -eq 1 ] && has "$err" "cannot be its own parent" && echo 0 || echo 1)
run $B --db "$E" add loose; run $B --db "$E" parent T4 T2
check "cross-epic parent (T4 has no epic, T2 is in E1): exit 1" $([ $rc -eq 1 ] && has "$err" "cross-epic parent" && echo 0 || echo 1)
run $B --db "$E" parent T3 none;                                       check "parent <T> none clears the link" $([ $rc -eq 0 ] && has "$out" "T3 no parent" && echo 0 || echo 1)
run $B --db "$E" add x --epic T1;                                      check "a T id for --epic: exit 1 with a hint" $([ $rc -eq 1 ] && has "$err" "T1 is a task id" && has "$err" "E3" && echo 0 || echo 1)
run $B --db "$E" add x --epic 1;                                       check "a bare number for --epic: exit 1" $([ $rc -eq 1 ] && has "$err" "bad epic id" && echo 0 || echo 1)
run $B --db "$E" add x --parent E1;                                    check "an E id for --parent: exit 1 with a hint" $([ $rc -eq 1 ] && has "$err" "E1 is an epic id" && has "$err" "epic get E1" && echo 0 || echo 1)
for v in "claim E1" "get E1" "status E1 done" "note E1 hi" "report E1 hi" "done E1 s" "review E1 s" "ask E1 q" "heartbeat E1" "dispatch E1" "wait E1" "parent E1 none"; do
  run $B --db "$E" $v;                                                 check "'$v' refuses an epic id (an epic is never leased): exit 1" $([ $rc -eq 1 ] && has "$err" "E1 is an epic id" && echo 0 || echo 1)
done
run $B --db "$E" brief
check "brief: one EPIC line per open epic with progress and path, then tasks that belong to no epic" $(has "$out" "EPIC E1 0/3 done $S/proj Ship it" && has "$out" "EPIC E2 0/0 done $S/not/there/yet (does not exist) Later" && ! has "$out" "EPIC E5" && has "$out" "T4 " && ! has "$out" "T1 rdy" && echo 0 || echo 1)

# --- edit: amend a spec, guarded by the lease -----------------------------------
use x; X=$BOARD
run $B --db "$X" epic add "Epic" --goal "g1"
run $B --db "$X" add "typo titel" --description "old text" --epic E1 --assignee zed
run $B --db "$X" edit T1 --title "typo title" --spec "new description" --priority 7
check "edit of a ready task lands and names the changed fields" $([ $rc -eq 0 ] && has "$out" "T1 edited title, description, priority #" && echo 0 || echo 1)
run $B --db "$X" get T1 --full
check "edit shows in the task and its timeline with old/new lengths" $(has "$out" "title: typo title" && has "$out" "priority 7" && has "$out" "new description" && has "$out" "title \"typo titel\"->\"typo title\", description 8->15, priority 100->7" && echo 0 || echo 1)
run $B --db "$X" edit T1 --assignee "";                                check "empty --assignee clears it" $([ $rc -eq 0 ] && has "$out" "T1 edited assignee" && echo 0 || echo 1)
before=$($B --db "$X" get T1 --full)
run $B --db "$X" edit T1 --title "typo title" --priority 7
check "edit with every value equal to the current one writes nothing" $([ $rc -eq 0 ] && [ "$out" = "T1 unchanged" ] && [ "$($B --db "$X" get T1 --full)" = "$before" ] && echo 0 || echo 1)
run $B --db "$X" edit T1;                                              check "edit with no field is a usage error, exit 1" $([ $rc -eq 1 ] && has "$err" "nothing to edit" && echo 0 || echo 1)
run $B --db "$X" edit T1 --title "  ";                                check "edit to an empty title: exit 1" $([ $rc -eq 1 ] && has "$err" "title cannot be empty" && echo 0 || echo 1)
run $B --db "$X" edit T1 --path "$S/proj/";                            check "edit --path on a task lands, trailing slash stripped" $([ $rc -eq 0 ] && has "$out" "T1 edited path" && echo 0 || echo 1)
WD=$($B --db "$X" add "workdir" --path "$S/proj/" --branch feat/x | cut -d' ' -f1)
run $B --db "$X" get $WD
check "add --path --branch: get shows the path without its trailing slash and the branch" $(has "$out" "path: $S/proj" && ! has "$out" "path: $S/proj/" && has "$out" "branch: feat/x" && echo 0 || echo 1)
NW=$($B --db "$X" add "no workdir" | cut -d' ' -f1)
run $B --db "$X" get $NW
check "a task added without them shows path: - and branch: HEAD" $(has "$out" "path: -" && has "$out" "branch: HEAD" && echo 0 || echo 1)
run $B --db "$X" add "bad" --branch 'a b';                             check "--branch with a space: exit 1, bad branch" $([ $rc -eq 1 ] && has "$err" "bad branch" && echo 0 || echo 1)
run $B --db "$X" edit $WD --branch HEAD;                               check "edit --branch lands" $([ $rc -eq 0 ] && has "$out" "$WD edited branch" && echo 0 || echo 1)
run $B --db "$X" get $WD --full;                                       check "the branch edit logs the old->new diff" $(has "$out" "branch \"feat/x\"->\"HEAD\"" && echo 0 || echo 1)
run $B --db "$X" edit $WD --path "";                                   check "empty --path clears it" $([ $rc -eq 0 ] && has "$out" "$WD edited path" && echo 0 || echo 1)
run $B --db "$X" get $WD;                                              check "a cleared path shows path: -" $(has "$out" "path: -" && echo 0 || echo 1)
run $B --db "$X" edit $WD --branch 'x~1';                              check "edit --branch with a bad ref: exit 1" $([ $rc -eq 1 ] && has "$err" "bad branch" && echo 0 || echo 1)
run $B --db "$X" edit T99 --title x;                                   check "edit of an unknown task: exit 1" $([ $rc -eq 1 ] && has "$err" "no such task T99" && echo 0 || echo 1)
run $B --actor=worker --db "$X" claim T1
before=$($B --db "$X" get T1 --full)
run $B --db "$X" edit T1 --goal "changed under the worker"
check "edit while working with a live lease is refused with the lease text, exit 1" $([ $rc -eq 1 ] && has "$err" "agentboard: T1 working by worker (lease" && has "$err" "manager: agentboard status T1 ready" && echo 0 || echo 1)
check "the refused edit leaves the row and timeline byte-identical" $([ "$($B --db "$X" get T1 --full)" = "$before" ] && echo 0 || echo 1)
$B --db "$X" status T1 ready >/dev/null
run $B --db "$X" edit T1 --goal "changed after release";               check "edit lands again once the manager frees the task" $([ $rc -eq 0 ] && has "$out" "T1 edited goal" && echo 0 || echo 1)

run $B --db "$X" epic get E1 >/dev/null
run $B --db "$X" edit E1 --title "Epic two" --path "$S/proj/"
check "edit of an epic: title and normalised path" $([ $rc -eq 0 ] && has "$out" "E1 edited title, path #" && echo 0 || echo 1)
run $B --db "$X" epic get E1
check "epic shows the new title, path and the edit event" $(has "$out" "title: Epic two" && has "$out" "path: $S/proj" && ! has "$out" "path: $S/proj/" && has "$out" "title \"Epic\"->\"Epic two\", path \"\"->\"$S/proj\"" && echo 0 || echo 1)
run $B --db "$X" edit E1 --branch feat/x;                              check "--branch on an epic: exit 1, names tasks" $([ $rc -eq 1 ] && has "$err" "--branch are for tasks" && echo 0 || echo 1)
run $B --db "$X" edit E1 --priority 3;                                 check "--priority on an epic: exit 1" $([ $rc -eq 1 ] && has "$err" "are for tasks" && echo 0 || echo 1)
run $B --db "$X" edit E1;                                              check "epic edit with no field: exit 1" $([ $rc -eq 1 ] && has "$err" "nothing to edit" && echo 0 || echo 1)
run $B --db "$X" edit E1 --title "Epic two";                           check "epic edit with equal values: unchanged" $([ "$out" = "E1 unchanged" ] && echo 0 || echo 1)

# review chain: three levels, the fourth reopen lands not_done and cascades one level
use chain; C=$BOARD
run $B --db "$C" add impl --status review;                                check "chain: level 1 (parentless, created at review so no review row is filed)" $rc
run $B --db "$C" add review --parent T1;                               check "chain: level 2 accepted" $rc
run $B --db "$C" add fix --parent T2;                                  check "chain: level 3 accepted" $rc
run $B --db "$C" add deeper --parent T3
check "chain: add under a level-3 row refused, exit 1, chain line" $([ $rc -eq 1 ] && has "$err" "chain too deep: T3 is level 3 and the new subtree spans 1, reaching level 4 (implement -> review -> fix); no fourth level" && echo 0 || echo 1)
run $B --db "$C" add loose; run $B --db "$C" parent T4 T3
check "chain: parent under a level-3 row refused, exit 1, chain line" $([ $rc -eq 1 ] && has "$err" "chain too deep: T3 is level 3 and the new subtree spans 1" && echo 0 || echo 1)
run $B --db "$C" parent T4 T1;                                         check "chain: parent under level 1 still fine" $rc
for i in 1 2 3; do run $B --db "$C" status T3 needs_fix "findings $i"; run $B --db "$C" status T3 working; done
run $B --db "$C" get T3;                                               check "reopens: 3/3 after three moves" $(has "$out" "reopens: 3/3" && ! has "$out" "cap hit" && echo 0 || echo 1)
run $B --db "$C" diff add T2 --body "the review row's own submission"
run $B --db "$C" status T2 review "parent at review"
run $B --db "$C" status T3 needs_fix "findings 4: the body"
check "cap: fourth move into needs_fix exits 1 naming the cap and not_done" $([ $rc -eq 1 ] && has "$err" "T3 reopen cap 3 hit" && has "$err" "not_done" && echo 0 || echo 1)
run $B --db "$C" get T3
check "cap: the row landed not_done, shows 4/3 cap hit, note carries the passed body" $(has "$out" "T3 not_done" && has "$out" "reopens: 4/3 (cap hit, stopped)" && has "$out" "findings 4: the body" && echo 0 || echo 1)
run $B --db "$C" get T2;                                               check "cap: the parent review row landed not_done with its own event" $(has "$out" "T2 not_done" && has "$out" "T3 hit the reopen cap 3" && echo 0 || echo 1)
run $B --db "$C" get T1;                                               check "cap: one level only, the implement row stays at review" $(has "$out" "T1 review" && echo 0 || echo 1)
run $B --db "$C" status T3 needs_fix "again";                          check "cap: a fifth try is refused too" $([ $rc -eq 1 ] && has "$err" "reopen cap" && echo 0 || echo 1)
run $B --db "$C" status T4 needs_fix x; run $B --db "$C" status T4 working; run $B --db "$C" get T4
check "reopens: 1/3 after one move" $(has "$out" "reopens: 1/3" && echo 0 || echo 1)
run $B --db "$C" status T4 not-done;                                   check "not_done spelling: not-done accepted" $(has "$out" "T4 not_done" && echo 0 || echo 1)
run $B --db "$C" ls;                                                   check "not_done is listed as nd before the working rows" $(has "$out" "nd" && echo 0 || echo 1)

# re-parenting counts the whole moved subtree; a closed parent is not cascaded onto
run $B --db "$C" add top;                                              check "subtree: a level-1 row" $rc
run $B --db "$C" add mid --parent T5;                                  check "subtree: a level-2 row under it" $rc
run $B --db "$C" add moved;                                            check "subtree: a loose row" $rc
run $B --db "$C" add moved-kid --parent T7;                            check "subtree: the loose row gets one child" $rc
run $B --db "$C" parent T7 T6
check "subtree: a row with one child under a level-2 row refused, exit 1, level named" $([ $rc -eq 1 ] && has "$err" "chain too deep: T6 is level 2 and the new subtree spans 2, reaching level 4" && echo 0 || echo 1)
run $B --db "$C" get T7;                                               check "subtree: the refused row did not move" $(has "$out" "parent:" && echo 1 || echo 0)
run $B --db "$C" parent T7 T5;                                         check "subtree: the same row under a level-1 row succeeds" $([ $rc -eq 0 ] && has "$out" "T7 parent T5" && echo 0 || echo 1)
run $B --db "$C" add rev2; run $B --db "$C" add fix2 --parent T9
for i in 1 2 3; do run $B --db "$C" status T10 needs_fix "f$i"; run $B --db "$C" status T10 working; done
run $B --db "$C" status T9 done "closed review"
run $B --db "$C" status T10 needs_fix "findings four"
check "closed parent: the capped child still exits 1 naming the cap" $([ $rc -eq 1 ] && has "$err" "T10 reopen cap 3 hit" && echo 0 || echo 1)
run $B --db "$C" get T9;                                              check "closed parent: a done parent stays done" $(has "$out" "T9 done" && ! has "$out" "hit the reopen cap" && echo 0 || echo 1)
run $B --db "$C" get T10;                                              check "closed parent: the child note names the parent and its status" $(has "$out" "T10 not_done" && has "$out" "parent T9 is done, left as it is" && has "$out" "findings four" && echo 0 || echo 1)

# a blocking parent that cascades to not_done must not keep needs=input
run $B --db "$C" add rev3; run $B --db "$C" add fix3 --parent T11
run $B --db "$C" block T11 "waiting on a human"
for i in 1 2 3; do run $B --db "$C" status T12 needs_fix "g$i"; run $B --db "$C" status T12 working; done
run $B --db "$C" status T12 needs_fix "capped"
run $B --db "$C" ls
check "cascade: a blocking parent lands not_done with needs cleared (no [input])" $(line=$(grep -E "^T11 " <<<"$out"); has "$line" "nd" && ! has "$line" "input" && echo 0 || echo 1)

# diffs: append-only rows a reviewer reads newest-first
use diffs; DF=$BOARD
run $B --db "$DF" add coded;                                           check "diffs: a task to record against" $rc
run $B --db "$DF" diff ls T1;                                          check "diff ls of a task with none prints (no diffs)" $([ $rc -eq 0 ] && [ "$out" = "(no diffs)" ] && echo 0 || echo 1)
out=$(printf 'line one\nline two\n' | $B --db "$DF" diff add T1 --body-file - 2>"$S/err"); rc=$?; err=$(<"$S/err")
check "diff add --body-file - reads stdin: D1, T1, round 1" $([ $rc -eq 0 ] && [ "$out" = "D1 (T1 round 1)" ] && echo 0 || echo 1)
run $B --db "$DF" diff get D1 --full
check "diff get --full: header with task, round, age, and the two-line body intact" $([ $rc -eq 0 ] && [[ "$(head -1 <<<"$out")" =~ ^D1\ T1\ round\ 1\ [0-9]+s ]] && [ "$(tail -n +2 <<<"$out")" = $'line one\nline two' ] && echo 0 || echo 1)
run $B --db "$DF" diff get D1;                                         check "diff get (clipped) folds newlines, still shows round" $([ $rc -eq 0 ] && has "$out" "round 1" && has "$out" "line one ⏎ line two" && echo 0 || echo 1)
run $B --db "$DF" status T1 needs_fix "r"; run $B --db "$DF" status T1 working
run $B --db "$DF" diff add T1 --body "- a bullet, hyphen first"
check "diff add --body accepts a leading hyphen; round 2 after one reopen" $([ $rc -eq 0 ] && [ "$out" = "D2 (T1 round 2)" ] && echo 0 || echo 1)
run $B --db "$DF" diff ls T1
check "diff ls: newest first, round and line count" $([ $rc -eq 0 ] && [[ "$(head -1 <<<"$out")" =~ ^D2\ round\ 2\ [0-9]+s\ 1\ lines$ ]] && [[ "$(tail -1 <<<"$out")" =~ ^D1\ round\ 1\ [0-9]+s\ 2\ lines$ ]] && echo 0 || echo 1)
run $B --db "$DF" get T1;                                              check "get shows the newest diff line" $(has "$out" "diff: D2 round 2 " && ! has "$out" "diff: D1" && echo 0 || echo 1)
run $B --db "$DF" diff add T1 --body "   
  ";                                                                   check "diff add: whitespace-only body refused, exit 1, says so" $([ $rc -eq 1 ] && has "$err" "the body is empty (whitespace only)" && echo 0 || echo 1)
run $B --db "$DF" diff add T99 --body x;                               check "diff add: unknown task exits 1" $([ $rc -eq 1 ] && has "$err" "no such task T99" && echo 0 || echo 1)
run $B --db "$DF" diff add nonsense --body x;                          check "diff add: bad task id exits 1" $([ $rc -eq 1 ] && has "$err" "bad task id" && echo 0 || echo 1)
run $B --db "$DF" diff add E1 --body x;                                check "diff add: an epic id exits 1 with the epic hint" $([ $rc -eq 1 ] && has "$err" "E1 is an epic id" && echo 0 || echo 1)
run $B --db "$DF" diff add T1;                                         check "diff add: no body source exits 1" $([ $rc -eq 1 ] && has "$err" "--body" && echo 0 || echo 1)
run $B --db "$DF" diff add T1 --body-file /nonexistent/x;              check "diff add: unreadable --body-file exits 1" $([ $rc -eq 1 ] && has "$err" "reading /nonexistent/x" && echo 0 || echo 1)
run $B --db "$DF" diff get D99;                                        check "diff get: unknown diff exits 1" $([ $rc -eq 1 ] && has "$err" "no such diff D99" && echo 0 || echo 1)
run $B --db "$DF" diff get T1;                                         check "diff get: a task id exits 1 with a hint" $([ $rc -eq 1 ] && has "$err" "not a diff id" && echo 0 || echo 1)
run $B --db "$DF" diff get nonsense;                                   check "diff get: a bad id exits 1" $([ $rc -eq 1 ] && has "$err" "bad diff id" && echo 0 || echo 1)
run $B --db "$DF" diff ls T99;                                         check "diff ls: unknown task exits 1" $([ $rc -eq 1 ] && has "$err" "no such task T99" && echo 0 || echo 1)
run $B --db "$DF" epic add ship; run $B --db "$DF" add inepic --epic E1; run $B --db "$DF" diff add T2 --body "x"
run $B --db "$DF" diff ls E1;                                          check "diff ls of an epic names the task of each diff" $([ $rc -eq 0 ] && [[ "$out" =~ ^D3\ T2\ round\ 1 ]] && echo 0 || echo 1)
run $B --db "$DF" status T1 dropped "gone"; run $B --db "$DF" diff ls T1
check "diffs survive the task being dropped" $([ $rc -eq 0 ] && has "$out" "D2 round 2" && has "$out" "D1 round 1" && echo 0 || echo 1)
run $B --db "$DF" wait T1 --kind diff --after 0 --timeout 1;           check "wait --kind diff is a known kind" $([ $rc -eq 0 ] && echo 0 || echo 1)

# the review gate and the auto-filed review row
use gate; G=$BOARD
run $B --db "$G" epic add "gate epic";                                 check "gate: an epic for the implement row" $rc
run $B --db "$G" add impl --epic E1 --path /tmp/agentboard-gate --branch feat/g --priority 7; run $B --db "$G" claim T1
run $B --db "$G" children T1;                                          check "gate: children before the move: none" $(has "$out" "has no children" && echo 0 || echo 1)
run $B --db "$G" review T1 "no diff yet"
check "gate: review with no diff since the claim is refused, exit 1, names diff add and the task" $([ $rc -eq 1 ] && has "$err" "T1 cannot move to review" && has "$err" "agentboard diff add T1" && echo 0 || echo 1)
run $B --db "$G" get T1;                                               check "gate: the refusal wrote nothing (still working, no child, no report)" $(has "$out" "T1 working" && ! has "$out" "no diff yet" && ! has "$out" "children:" && echo 0 || echo 1)
run $B --db "$G" diff add T1 --body "first diff"
run $B --db "$G" review T1 $'the report\nsecond line';                  check "gate: the same move succeeds once a diff exists, exit 0, names the review row" $([ $rc -eq 0 ] && has "$out" "T1 review #" && has "$out" "(review row T2)" && echo 0 || echo 1)
run $B --db "$G" children T1;                                          check "autofile: exactly one child, ready, titled Review impl" $([ $rc -eq 0 ] && [ "$(grep -c . <<<"$out")" = 1 ] && has "$out" "T2 rdy" && has "$out" "Review impl" && echo 0 || echo 1)
run $B --db "$G" get T2 --full
check "autofile: the child inherits epic/path/branch/priority, no assignee" $(has "$out" "T2 ready - priority 7" && has "$out" "epic: E1 gate epic" && has "$out" "path: /tmp/agentboard-gate" && has "$out" "branch: feat/g" && has "$out" "parent: T1" && echo 0 || echo 1)
check "autofile: the report is copied verbatim and the newest diff id is named" $(has "$out" $'the report\nsecond line' && has "$out" "Newest diff: D1" && has "$out" "never close T1" && echo 0 || echo 1)
run $B --db "$G" get T1;                                               check "autofile: the implement row logs a note naming the review row" $(has "$out" "review row T2 filed" && echo 0 || echo 1)
run $B --db "$G" status T1 needs_fix "findings"; run $B --db "$G" status T1 working
run $B --db "$G" review T1 "second submission"
check "gate: after needs_fix and a new working move the OLD diff no longer satisfies it" $([ $rc -eq 1 ] && has "$err" "agentboard diff add T1" && echo 0 || echo 1)
run $B --db "$G" diff add T1 --body "second diff"; run $B --db "$G" review T1 "second submission"
check "gate: a diff recorded after the new claim satisfies it" $([ $rc -eq 0 ] && has "$out" "(review row T2)" && echo 0 || echo 1)
run $B --db "$G" children T1;                                          check "autofile: a resubmission files no second review row" $([ "$(grep -c . <<<"$out")" = 1 ] && echo 0 || echo 1)
run $B --db "$G" get T2;                                               check "autofile: the existing review row gets a note naming the new diff, description untouched" $(has "$out" "T1 submitted again: newest diff D2 (round 2)" && has "$out" "Newest diff: D1" && echo 0 || echo 1)
run $B --db "$G" status T1 needs_fix "round three"; run $B --db "$G" edit T1 --title "impl renamed"; run $B --db "$G" status T1 working
run $B --db "$G" diff add T1 --body "third diff"; run $B --db "$G" review T1 "third submission"
check "autofile: renaming the implement row between rounds still files no second review row" $([ $rc -eq 0 ] && has "$out" "(review row T2)" && echo 0 || echo 1)
run $B --db "$G" children T1;                                          check "autofile: children stays one row after the rename" $([ "$(grep -c . <<<"$out")" = 1 ] && has "$out" "T2 " && echo 0 || echo 1)
run $B --db "$G" get T2;                                               check "autofile: the rename's resubmission note lands on the original review row" $(has "$out" "T1 submitted again: newest diff D3 (round 3)" && echo 0 || echo 1)
run $B --db "$G" add fix --parent T2; run $B --db "$G" claim T3
run $B --db "$G" children T3;                                          check "autofile: a fix row has no children before" $(has "$out" "has no children" && echo 0 || echo 1)
run $B --db "$G" diff add T3 --body "fix diff"; run $B --db "$G" review T3 "fixed"
check "autofile: a parented row moved to review files nothing" $([ $rc -eq 0 ] && ! has "$out" "review row" && echo 0 || echo 1)
run $B --db "$G" children T3;                                          check "autofile: and still has no children after" $(has "$out" "has no children" && echo 0 || echo 1)
run $B --db "$G" ls --limit 50;                                        check "autofile: nothing beyond T1..T3 exists" $(! has "$out" "T4" && echo 0 || echo 1)

# a board file from before epics is refused, untouched, with the way out (sqlite only;
# the surreal build's wrong-kind and schema refusals are phase (n)/(o) of surreal.sh)
if [ "$BACKEND" = sqlite ]; then
O=$S/old.db
python3 - "$O" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript("CREATE TABLE tasks(id INTEGER PRIMARY KEY, title TEXT, parent INTEGER); INSERT INTO tasks(title) VALUES('old');")
c.commit()
PY
before=$(sha1sum "$O" | cut -d' ' -f1)
run $B --db "$O" ls
check "old board: exit 1, says it predates epics and how to move it aside" $([ $rc -eq 1 ] && has "$err" "predates epics" && has "$err" "mv $O $O.old" && echo 0 || echo 1)
run $B --db "$O" add nope;                                             check "old board: writes refused too" $([ $rc -eq 1 ] && has "$err" "predates epics" && echo 0 || echo 1)
check "old board file is byte-identical afterwards (never half-migrated)" $([ "$(sha1sum "$O" | cut -d' ' -f1)" = "$before" ] && [ ! -e "$O-wal" ] && echo 0 || echo 1)
R=$S/noreopen.db
python3 - "$R" <<'PY'
import sqlite3, sys
c = sqlite3.connect(sys.argv[1])
c.executescript("CREATE TABLE tasks(id INTEGER PRIMARY KEY, title TEXT, epic_id INTEGER, parent_id INTEGER, path TEXT, branch TEXT); CREATE TABLE epics(id INTEGER PRIMARY KEY); CREATE TABLE events(seq INTEGER PRIMARY KEY, task_id INTEGER, epic_id INTEGER);")
c.commit()
PY
run $B --db "$R" ls
check "board without reopen_count: exit 1, one-line move-it-aside refusal" $([ $rc -eq 1 ] && has "$err" "predates task reopen_count" && has "$err" "mv $R $R.old" && echo 0 || echo 1)
run $B --db "$O" path;                                                 check "'path' still works on an old board (it names the file to move)" $([ $rc -eq 0 ] && [ "$out" = "$O" ] && echo 0 || echo 1)
P=$S/surface.db
run $B --db "$P" ls
python3 - "$P" <<'PYDB'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as c:
    c.execute("UPDATE board_stamp SET generation=99, opened_by='future tools=99 commit=future'")
PYDB
run $B --db "$P" add refused
check "surface mismatch: exit 1, one line names both stamps and file" $([ $rc -eq 1 ] && [ "$(wc -l <<<"$err")" = 1 ] && has "$err" "board generation=99 future tools=99 commit=future" && has "$err" "client generation=2 agentboard" && has "$err" "file $P" && echo 0 || echo 1)
python3 - "$P" <<'PYDB'
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as c:
    c.execute("DROP TABLE board_stamp")
PYDB
before=$(sha1sum "$P" | cut -d' ' -f1)
run $B --db "$P" add refused
check "unstamped board: refused before DDL, file unchanged" $([ $rc -eq 1 ] && has "$err" "board unstamped" && [ "$(sha1sum "$P" | cut -d' ' -f1)" = "$before" ] && echo 0 || echo 1)

fi

# manager: --tmux is refused by name; a bad socket is one line, exit 1. HOME and the
# socket variables point into $S, so neither check can reach the live herdr session.
mgr() { run env -u AGENTBOARD_HERDR_SOCK -u HERDR_SOCKET XDG_CONFIG_HOME="$S/xdg" HOME="$S/home" "$@"; }
mgr "$B" manager --tmux status;                                                     check "manager --tmux: refused by name, exit 1" $([ $rc -eq 1 ] && [ "$err" = "agentboard: --tmux: no tmux backend yet; only --herdr is implemented" ] && echo 0 || echo 1)
mgr "$B" manager --herdr --tmux status;                                             check "manager --herdr --tmux: clap conflict, exit 1" $([ $rc -eq 1 ] && has "$err" "agentboard: " && echo 0 || echo 1)
mgr "$B" manager agent list;                                           check "manager with no socket: exit 1, one line naming all four paths" $([ $rc -eq 1 ] && [ "$(wc -l <<<"$err")" = 1 ] && has "$err" "\$AGENTBOARD_HERDR_SOCK" && has "$err" "\$HERDR_SOCKET" && has "$err" "$S/xdg/herdr/herdr.sock" && has "$err" "$S/home/.config/herdr/herdr.sock" && echo 0 || echo 1)
mgr env AGENTBOARD_HERDR_SOCK="$S/none.sock" "$B" manager agent list;  check "manager with a named socket that is missing: exit 1, names the variable and path" $([ $rc -eq 1 ] && [ "$(wc -l <<<"$err")" = 1 ] && has "$err" "\$AGENTBOARD_HERDR_SOCK is set but $S/none.sock does not exist" && echo 0 || echo 1)
mgr "$B" manager status --bogus;                                                    check "manager status rejects unknown flags, exit 1" $([ $rc -eq 1 ] && has "$err" "'--bogus'" && echo 0 || echo 1)
mgr "$B" manager agent read coder --source bogus;                                   check "manager agent read --source is a closed set, exit 1" $([ $rc -eq 1 ] && has "$err" "possible values" && echo 0 || echo 1)

# no args = MCP server on stdio, not a help print
init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cli.sh","version":"0"}}}'
use mcp
for mode in "" "serve"; do
  reply=$(printf '%s\n' "$init" | timeout 5 $B $mode --db "$BOARD" 2>/dev/null | head -1)
  check "no-arg/serve (\"$mode\") answers JSON-RPC initialize" $(has "$reply" '"result"' && has "$reply" 'protocolVersion' && ! has "$reply" Usage && echo 0 || echo 1)
done

# the build stamp: the same one line from the CLI verb and from initialize, with the
# tool count derived from the source markers (never written down)
run $B board-build;                                                    check "board-build: one line naming version, tools, commit, backend" $([ $rc -eq 0 ] && [ "$(wc -l <<<"$out")" = 1 ] && has "$out" "agentboard 0.1.0 tools=" && has "$out" "commit=" && has "$out" "backend=$BACKEND" && echo 0 || echo 1)
markers=$(grep -c '^[[:space:]]*#\[tool(' src/server.rs)
check "board-build: its tool count equals the #[tool( markers in src/server.rs" $(has "$out" "tools=$markers " && echo 0 || echo 1)

echo "$pass/$total passed (backend: $BACKEND)"
[ "$pass" -eq "$total" ]
