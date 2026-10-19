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

# no args = MCP server on stdio, not a help print
init='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"cli.sh","version":"0"}}}'
for mode in "" "serve"; do
  reply=$(printf '%s\n' "$init" | timeout 5 $B $mode --db "$S/mcp.db" 2>/dev/null | head -1)
  check "no-arg/serve (\"$mode\") answers JSON-RPC initialize" $(has "$reply" '"result"' && has "$reply" 'protocolVersion' && ! has "$reply" Usage && echo 0 || echo 1)
done

echo "$pass/$total passed"
[ "$pass" -eq "$total" ]
