#!/usr/bin/env bash
# dispatch contract with stub `herdr` and `tmux` on PATH: refusal, --print,
# --force, target resolution, herdr failure -> tmux fallback, herdr hang ->
# deadline kill. Shell, not mcp_client.py: it needs fake binaries, exit codes,
# stderr and wall time, none of which the MCP text channel carries.
set -u
cd "$(dirname "$0")/.."
B=$PWD/target/release/agentboard
S=$(mktemp -d /tmp/agentboard-dispatch.XXXXXX)
trap 'rm -rf "$S"' EXIT
mkdir "$S/bin"
export AGENTBOARD_DB=$S/d.db AGENTBOARD_ACTOR=boss LOG=$S/calls
# herdr: HERDR_MODE ok|fail|hang (hang = a shell with a backgrounded sleep, so a
# kill that spares descendants leaves a live process; HERDR_AGENTS lists live agents).
# tmux: TMUX_MODE ok|fail|hang|down, TMUX_LIVE names the sessions that exist. Every call is logged.
cat >"$S/bin/herdr" <<'STUB'
#!/bin/sh
echo "herdr $*" >>"$LOG"
case "${HERDR_MODE:-ok}" in
  fail) echo "no such pane" >&2; exit 1 ;;
  hang) echo $$ >"$(dirname "$LOG")/herdr.pid"; sleep 30 & echo $! >"$(dirname "$LOG")/sleep.pid"; wait ;;
esac
if [ "$1 $2" = "agent list" ]; then   # HERDR_AGENTS="name:kind name:kind"
  agents=""
  for pair in ${HERDR_AGENTS:-}; do agents="$agents{\"name\":\"${pair%%:*}\",\"agent\":\"${pair##*:}\"},"; done
  echo "{\"result\":{\"agents\":[${agents%,}]}}"
fi
exit 0
STUB
cat >"$S/bin/tmux" <<'STUB'
#!/bin/sh
echo "tmux $*" >>"$LOG"
# down: no server at all. Otherwise liveness answers (TMUX_LIVE="a b" = only those
# sessions exist; unset = every name exists), and fail/hang hit only the transport.
[ "${TMUX_MODE:-ok}" = down ] && { echo "no server running" >&2; exit 1; }
case "$1" in
  has-session)
    [ -z "${TMUX_LIVE+x}" ] && exit 0
    case " $TMUX_LIVE " in *" $3 "*) exit 0 ;; esac
    echo "can't find session: $3" >&2; exit 1 ;;
  send-keys|capture-pane)
    [ "${TMUX_MODE:-ok}" = fail ] && { echo "no server running" >&2; exit 1; }
    [ "${TMUX_MODE:-ok}" = hang ] && { echo $$ >"$(dirname "$LOG")/tmux.pid"; sleep 30; } ;;
esac
exit 0
STUB
chmod +x "$S/bin/herdr" "$S/bin/tmux"
export PATH=$S/bin:$PATH
export AGENTBOARD_DELIVER_TIMEOUT_MS=300

pass=0 total=0 rc=0 out="" err=""
run() { out=$("$@" 2>"$S/err"); rc=$?; err=$(<"$S/err"); }
check() { total=$((total + 1)); if [ "$2" -eq 0 ]; then pass=$((pass + 1)); echo "PASS $1"; else echo "FAIL $1 :: rc=$rc out=[$out] err=[$err]"; fi; }
has() { [[ "$1" == *"$2"* ]]; }
gone() { for _ in 1 2 3 4 5 6 7 8 9 10; do kill -0 "$(<"$1")" 2>/dev/null || return 0; sleep 0.1; done; return 1; }   # pid file names a dead process
calls() { grep -c "^$1" "$LOG" 2>/dev/null || true; }
events() { $B get "$1" --full | grep -c " $2 [0-9]*s:" || true; }   # events of one kind on a task
: >"$LOG"

$B register w1 >/dev/null; $B register w2 >/dev/null
$B add t1 --assignee w1 >/dev/null                      # T1 ready, registered assignee
$B add t2 --assignee w1 --status working >/dev/null     # T2 working
$B add t3 --assignee ghost >/dev/null                   # T3 ready, unregistered assignee
$B add t4 >/dev/null                                    # T4 ready, no assignee

# refusal: not ready, no --force
run $B dispatch T2
check "non-ready task: exit 1, says not ready" $([ $rc -eq 1 ] && has "$err" "not ready" && echo 0 || echo 1)
check "refusal wrote no dispatch event and ran no stub" $([ "$(events T2 dispatch)" = 0 ] && [ ! -s "$LOG" ] && echo 0 || echo 1)

# --print touches nothing, even with --force
run $B dispatch T2 --force --print
check "--print --force: exit 0, prints the line" $([ $rc -eq 0 ] && has "$out" "Claim T2, then task_get id=2" && echo 0 || echo 1)
check "--print: no stub run, no event, status unchanged" $([ ! -s "$LOG" ] && [ "$(events T2 dispatch)" = 0 ] && [ "$(events T2 edit)" = 0 ] && $B get T2 | grep -q "^T2 working" && echo 0 || echo 1)

# --force on a working task: reset through ready, then send, one dispatch event
run $B dispatch T2 --force
check "--force: exit 0, reports the reset and the path" $([ $rc -eq 0 ] && has "$out" "--force: T2 working -> ready" && has "$out" "dispatched to w1 via herdr" && echo 0 || echo 1)
check "--force: task is ready, one edit event, one dispatch event" $([ "$(events T2 edit)" = 1 ] && [ "$(events T2 dispatch)" = 1 ] && $B get T2 | grep -q "^T2 ready" && echo 0 || echo 1)
check "--force: herdr got exactly one prompt" $([ "$(calls 'herdr agent prompt w1')" = 1 ] && echo 0 || echo 1)

# plain success on a ready task: one dispatch event naming the path
: >"$LOG"
run $B dispatch T1
check "ready task: exit 0 via herdr" $([ $rc -eq 0 ] && has "$out" "dispatched to w1 via herdr:" && echo 0 || echo 1)
check "exactly one dispatch event, body names target and path" $([ "$(events T1 dispatch)" = 1 ] && $B get T1 --full | grep -q "dispatch [0-9]*s: w1 via herdr$" && echo 0 || echo 1)
check "tmux gets no keys on herdr success (only liveness probes)" $([ "$(calls 'tmux send-keys')" = 0 ] && echo 0 || echo 1)

# target resolution: registered assignee wins, then --to, else an error
run $B dispatch T3 --to w2
check "unregistered assignee falls to --to w2" $([ $rc -eq 0 ] && has "$out" "dispatched to w2 via herdr" && echo 0 || echo 1)
$B add t5 --assignee w1 >/dev/null
run $B dispatch T5 --to w2
check "registered assignee beats --to" $([ $rc -eq 0 ] && has "$out" "dispatched to w1 via" && echo 0 || echo 1)
run $B dispatch T4 --to nosuch
check "--to unregistered: exit 1" $([ $rc -eq 1 ] && has "$err" "no registered session" && echo 0 || echo 1)
run $B dispatch T4
check "no assignee, no --to: exit 1" $([ $rc -eq 1 ] && has "$err" "no registered assignee" && echo 0 || echo 1)
check "target errors wrote no dispatch event" $([ "$(events T4 dispatch)" = 0 ] && echo 0 || echo 1)

# herdr fails -> tmux, and the reason is not lost
$B add t6 --assignee w1 >/dev/null; : >"$LOG"
HERDR_MODE=fail run $B dispatch T6
check "herdr fails: falls back to tmux, line names the reason" $([ $rc -eq 0 ] && has "$out" "via tmux (herdr: no such pane)" && echo 0 || echo 1)
check "event body names the reason" $($B get T6 --full | grep -q "dispatch [0-9]*s: w1 via tmux (herdr: no such pane)$" && echo 0 || echo 1)
check "tmux received the text" $([ "$(calls 'tmux send-keys -t w1')" -ge 1 ] && echo 0 || echo 1)

# herdr hangs -> killed at the deadline -> tmux; wall time bounded
$B add t7 --assignee w1 >/dev/null
t0=$(date +%s%N)
HERDR_MODE=hang run $B dispatch T7
ms=$(( ($(date +%s%N) - t0) / 1000000 ))
check "herdr hang: killed at the deadline, tmux wins (${ms}ms, limit 300ms + slack)" $([ $rc -eq 0 ] && [ $ms -lt 2500 ] && has "$out" "via tmux (herdr: timed out after 300ms)" && echo 0 || echo 1)
check "hang: the herdr stub shell AND its backgrounded sleep are dead (process group killed)" $(gone "$S/herdr.pid" && gone "$S/sleep.pid" && echo 0 || echo 1)
check "hang reason is in the event body" $($B get T7 --full | grep -q "via tmux (herdr: timed out after 300ms)$" && echo 0 || echo 1)

# both fail -> exit 1 with both reasons, and no dispatch event
$B add t8 --assignee w1 >/dev/null
HERDR_MODE=fail TMUX_MODE=fail run $B dispatch T8
check "both paths fail: exit 1 naming both" $([ $rc -eq 1 ] && has "$err" "herdr: no such pane" && has "$err" "no server running" && echo 0 || echo 1)
check "failed delivery wrote no dispatch event" $([ "$(events T8 dispatch)" = 0 ] && echo 0 || echo 1)

# --force when delivery then fails: the reset stays (documented), nothing is sent or logged as dispatched
$B add t9 --assignee w1 --status working >/dev/null
HERDR_MODE=fail TMUX_MODE=fail run $B dispatch T9 --force
check "--force + both fail: exit 1" $([ $rc -eq 1 ] && has "$err" "dispatch to w1 failed" && echo 0 || echo 1)
check "--force + both fail: task left ready, one edit event 'dispatch --force', zero dispatch events" $([ "$(events T9 edit)" = 1 ] && [ "$(events T9 dispatch)" = 0 ] && $B get T9 --full | grep -q "edit [0-9]*s: dispatch --force$" && $B get T9 | grep -q "^T9 ready" && echo 0 || echo 1)

# tmux is bounded too: a wedged tmux ends dispatch (and tail/send) at the deadline
$B add t10 --assignee w1 >/dev/null
t0=$(date +%s%N)
HERDR_MODE=fail TMUX_MODE=hang run $B dispatch T10
ms=$(( ($(date +%s%N) - t0) / 1000000 ))
check "tmux hang: dispatch exits 1 at the deadline (${ms}ms) naming it" $([ $rc -eq 1 ] && [ $ms -lt 2500 ] && has "$err" "timed out after 300ms" && echo 0 || echo 1)
check "tmux hang: stub dead, no dispatch event" $(gone "$S/tmux.pid" && [ "$(events T10 dispatch)" = 0 ] && echo 0 || echo 1)
t0=$(date +%s%N)
TMUX_MODE=hang run $B tail w1
ms=$(( ($(date +%s%N) - t0) / 1000000 ))
check "tmux hang: tail exits 1 at the deadline (${ms}ms)" $([ $rc -eq 1 ] && [ $ms -lt 2500 ] && has "$err" "tmux: timed out" && echo 0 || echo 1)
TMUX_MODE=hang run $B send w1 hi
check "tmux hang: send exits 1 at the deadline" $([ $rc -eq 1 ] && has "$err" "tmux: timed out" && echo 0 || echo 1)

# `sessions` is bounded by the same runner and knob: a herdr whose child holds
# stdout must not stall it (the old ad-hoc runner joined the reader and waited
# for the descendant, 30s here), and the stub tree dies with the deadline.
t0=$(date +%s%N)
TMUX_LIVE="" HERDR_MODE=hang run $B sessions
ms=$(( ($(date +%s%N) - t0) / 1000000 ))
check "sessions with a stdout-holding herdr: returns at the deadline (${ms}ms), exit 0, rows intact" $([ $rc -eq 0 ] && [ $ms -lt 2500 ] && has "$out" "w1" && echo 0 || echo 1)
check "sessions: herdr stub shell AND its sleep are dead" $(gone "$S/herdr.pid" && gone "$S/sleep.pid" && echo 0 || echo 1)
HERDR_MODE=fail run $B sessions
check "sessions with a failing herdr still lists the registry" $([ $rc -eq 0 ] && has "$out" "w1" && echo 0 || echo 1)

# --- stale rows: a registry row is a claim, not proof of a live pane ----------
# w1 and w2 are live tmux sessions; ghost* have rows but no pane anywhere; hv is
# alive only as a herdr agent of the same kind.
export TMUX_LIVE="w1 w2"
$B register ghost >/dev/null; $B register ghost2 >/dev/null; $B register hv --agent claude >/dev/null
$B add s1 --assignee ghost >/dev/null; $B add s2 --assignee ghost >/dev/null; $B add s3 --assignee ghost >/dev/null
$B add s4 --assignee nobody >/dev/null; $B add s5 --assignee ghost >/dev/null; : >"$LOG"
S1=$($B ls --limit 50 | awk '$0 ~ / s1$/ {print $1}'); S2=$($B ls --limit 50 | awk '$0 ~ / s2$/ {print $1}')
S3=$($B ls --limit 50 | awk '$0 ~ / s3$/ {print $1}'); S4=$($B ls --limit 50 | awk '$0 ~ / s4$/ {print $1}')
S5=$($B ls --limit 50 | awk '$0 ~ / s5$/ {print $1}')

run $B dispatch "$S1" --to w2
check "stale assignee row cannot capture the dispatch: --to w2 gets it" $([ $rc -eq 0 ] && has "$out" "dispatched to w2 via herdr" && ! has "$(<"$LOG")" "prompt ghost" && echo 0 || echo 1)
run $B dispatch "$S2"
check "stale assignee, no --to: exit 1, names the assignee and the stale row" $([ $rc -eq 1 ] && has "$err" '"ghost"' && has "$err" "stale" && has "$err" "sessions prune" && echo 0 || echo 1)
check "stale refusal sent nothing and logged nothing" $([ "$(calls 'herdr agent prompt ghost')" = 0 ] && [ "$(events "$S2" dispatch)" = 0 ] && echo 0 || echo 1)
run $B dispatch "$S3" --to ghost2
check "stale assignee AND stale --to: error names both" $([ $rc -eq 1 ] && has "$err" '"ghost"' && has "$err" '"ghost2"' && has "$err" "not live" && echo 0 || echo 1)
run $B dispatch "$S4" --to ghost2
check "unregistered assignee + stale --to: exit 1 'not live'" $([ $rc -eq 1 ] && has "$err" "not live" && echo 0 || echo 1)
HERDR_AGENTS="hv:claude" run $B dispatch "$S5" --to hv
check "herdr-only pane (same kind) counts as live" $([ $rc -eq 0 ] && has "$out" "dispatched to hv via herdr" && echo 0 || echo 1)
$B add s6 --assignee hv >/dev/null; S6=$($B ls --limit 50 | awk '$0 ~ / s6$/ {print $1}')
HERDR_AGENTS="hv:codex" run $B dispatch "$S6" --to w1
check "herdr agent of a different kind does not vouch for the row" $([ $rc -eq 0 ] && has "$out" "dispatched to w1 via" && echo 0 || echo 1)

# prune
HERDR_AGENTS="hv:claude" run $B sessions prune --dry-run
check "prune --dry-run lists exactly the dead rows" $([ $rc -eq 0 ] && has "$out" "would remove ghost (agent)" && has "$out" "would remove ghost2 (agent)" && ! has "$out" "would remove w1" && ! has "$out" "would remove hv" && has "$out" "would prune 2, kept 3" && echo 0 || echo 1)
run $B sessions;                                                       check "--dry-run deleted nothing" $(has "$out" "ghost " && has "$out" "ghost2 " && echo 0 || echo 1)
TMUX_MODE=down HERDR_MODE=fail run $B sessions prune
check "prune refuses when neither tmux nor herdr answers" $([ $rc -eq 1 ] && has "$err" "neither tmux nor herdr answers" && echo 0 || echo 1)
run $B sessions;                                                       check "refused prune deleted nothing" $(has "$out" "ghost " && has "$out" "hv " && echo 0 || echo 1)
HERDR_MODE=fail run $B sessions prune --dry-run
check "herdr down but tmux up: prune proceeds and says it checked tmux only" $([ $rc -eq 0 ] && has "$out" "herdr unreachable" && echo 0 || echo 1)
HERDR_AGENTS="hv:claude" run $B sessions prune
check "prune removes exactly the dead rows" $([ $rc -eq 0 ] && has "$out" "removed ghost (agent)" && has "$out" "removed ghost2 (agent)" && has "$out" "pruned 2, kept 3" && echo 0 || echo 1)
HERDR_AGENTS="hv:claude" run $B sessions
check "live rows (tmux w1 w2, herdr hv) survive; dead rows are gone" $(has "$out" "w1 " && has "$out" "w2 " && has "$out" "hv " && ! has "$out" "ghost" && echo 0 || echo 1)
HERDR_AGENTS="hv:claude" run $B sessions prune
check "second prune: nothing to prune" $(has "$out" "nothing to prune (3 live)" && echo 0 || echo 1)
$B register tmp >/dev/null
run $B unregister tmp;                                                 check "unregister removes one row" $([ $rc -eq 0 ] && has "$out" "ok session tmp removed" && echo 0 || echo 1)
run $B unregister tmp;                                                 check "unregister of an unknown name exits 1" $([ $rc -eq 1 ] && has "$err" "no registered session" && echo 0 || echo 1)
unset TMUX_LIVE

echo "$pass/$total passed"
[ "$pass" -eq "$total" ]
