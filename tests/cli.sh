#!/usr/bin/env bash
# CLI contract: exit codes, stderr text, flag spellings, no ANSI. These are
# the checks mcp_client.py cannot make (it reads tool text, not stderr/exit).
set -u
cd "$(dirname "$0")/.."
B=$PWD/target/release/agentboard
S=$(mktemp -d /tmp/agentboard-cli.XXXXXX)
trap 'rm -rf "$S"' EXIT
# Every command names its DB; this default only catches one that forgets.
export AGENTBOARD_DB=$S/stray.db AGENTBOARD_ACTOR=cli
unset NO_COLOR
pass=0 total=0
rc=0 out="" err=""

run() { out=$("$@" 2>"$S/err"); rc=$?; err=$(<"$S/err"); }
check() { # check <name> <condition-result: 0 = ok>
  total=$((total + 1))
  if [ "$2" -eq 0 ]; then pass=$((pass + 1)); echo "PASS $1"; else echo "FAIL $1 :: rc=$rc out=[$out] err=[$err]"; fi
}
has() { [[ "$1" == *"$2"* ]]; }

D=$S/a.db
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
run $B --db "$S/empty.db" claim;                                       check "id-less claim on an empty board: exit 0, no ready tasks" $([ $rc -eq 0 ] && has "$out" "no ready tasks" && echo 0 || echo 1)
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

# --- epics: own table, own ids, project path -----------------------------------
E=$S/e.db
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
X=$S/x.db
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
run $B --db "$X" edit T1 --path /x;                                    check "--path on a task: exit 1" $([ $rc -eq 1 ] && has "$err" "--path is for epics" && echo 0 || echo 1)
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
run $B --db "$X" edit E1 --priority 3;                                 check "--priority on an epic: exit 1" $([ $rc -eq 1 ] && has "$err" "are for tasks" && echo 0 || echo 1)
run $B --db "$X" edit E1;                                              check "epic edit with no field: exit 1" $([ $rc -eq 1 ] && has "$err" "nothing to edit" && echo 0 || echo 1)
run $B --db "$X" edit E1 --title "Epic two";                           check "epic edit with equal values: unchanged" $([ "$out" = "E1 unchanged" ] && echo 0 || echo 1)

# a board file from before epics is refused, untouched, with the way out
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
run $B --db "$O" path;                                                 check "'path' still works on an old board (it names the file to move)" $([ $rc -eq 0 ] && [ "$out" = "$O" ] && echo 0 || echo 1)

# no args = MCP server on stdio, not a help print
init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cli.sh","version":"0"}}}'
for mode in "" "serve"; do
  reply=$(printf '%s\n' "$init" | timeout 5 $B $mode --db "$S/mcp.db" 2>/dev/null | head -1)
  check "no-arg/serve (\"$mode\") answers JSON-RPC initialize" $(has "$reply" '"result"' && has "$reply" 'protocolVersion' && ! has "$reply" Usage && echo 0 || echo 1)
done

echo "$pass/$total passed"
[ "$pass" -eq "$total" ]
