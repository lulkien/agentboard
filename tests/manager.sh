#!/usr/bin/env bash
# `manager up|down|renew` against tests/herdr_stub.py and a throwaway board: the request
# order each composite sends, the board rows it writes by name, and the working guard.
# Shell, not tests/manager.rs: the composites write the board, which needs board.sh's
# throwaway server. The stub is a socket in $S, so nothing reaches a real herdr.
set -u
cd "$(dirname "$0")/.."
B=${AGENTBOARD_TEST_BIN:-$PWD/target/release/agentboard}
S=$(mktemp -d /tmp/agentboard-manager.XXXXXX)
source tests/board.sh
STUB_PID=
stop_stub() { [ -z "$STUB_PID" ] || kill "$STUB_PID" 2>/dev/null; STUB_PID=; rm -f "$S/herdr.sock"; }
trap 'stop_stub; board_stop; rm -rf "$S"' EXIT
use m
export AGENTBOARD_DB=$BOARD AGENTBOARD_ACTOR=mgr AGENTBOARD_HERDR_TIMEOUT_MS=3000
# No ambient herdr socket can be reached; the stub's path is the only one named.
unset HERDR_SOCKET
export HOME=$S/home XDG_CONFIG_HOME=$S/home/xdg
mkdir -p "$HOME/.claude" "$S/repo"
echo '{}' >"$HOME/.claude/coding-mcp.json"; echo '{}' >"$HOME/.claude/reviewer-mcp.json"
# Executables are fixtures: herdr is stubbed, so no real CLI is launched.
mkdir -p "$S/bin" "$HOME/.codex"
for kind in claude opencode codex; do
  printf '#!/bin/sh\nexit 0\n' >"$S/bin/$kind"
  chmod +x "$S/bin/$kind"
done
for role in coder reviewer analyzer manager; do echo '#' >"$HOME/.codex/$role.config.toml"; done
unset CODEX_HOME
export PATH="$S/bin:$PATH"
pass=0 total=0 rc=0 out="" err=""
run() { out=$("$@" 2>"$S/err"); rc=$?; err=$(<"$S/err"); }
check() { total=$((total + 1)); if [ "$2" -eq 0 ]; then pass=$((pass + 1)); echo "PASS $1"; else echo "FAIL $1 :: rc=$rc out=[$out] err=[$err]"; fi; }
has() { [[ "$1" == *"$2"* ]]; }
# stub [AGENTS_JSON]: a fresh stub with an empty log, agents live as given.
stub() {
  stop_stub; : >"$S/log"; echo "${1:-[]}" >"$S/agents.json"
  python3 tests/herdr_stub.py "$S/herdr.sock" "$S/log" "$S/agents.json" & STUB_PID=$!
  for _ in $(seq 1 50); do [ -S "$S/herdr.sock" ] && break; sleep 0.1; done
  export AGENTBOARD_HERDR_SOCK=$S/herdr.sock
}
methods() { python3 -c 'import json,sys;print(" ".join(json.loads(l)["method"] for l in open(sys.argv[1])))' "$S/log"; }
req() { python3 -c 'import json,sys;[print(json.dumps(json.loads(l)["params"],separators=(",",":"),sort_keys=True)) for l in open(sys.argv[1]) if json.loads(l)["method"]==sys.argv[2]]' "$S/log" "$1"; }
M() { run $B manager "$@"; }
rows() { $B sessions 2>&1; }

stub
M up --repo "$S/repo"
echo "up: $out"
check "up exits 0 and prints one line naming tab, panes and agents" $([ $rc -eq 0 ] && [ "$(grep -c . <<<"$out")" = 1 ] && has "$out" "s:t1" && has "$out" "coder pane s:p1 (claude)" && has "$out" "reviewer pane s:p2 (opencode)" && echo 0 || echo 1)
check "up sends tab, split, then each start followed by its status read" $([ "$(methods)" = "agent.list tab.create pane.split agent.start agent.list agent.start agent.list" ] && echo 0 || echo 1)
check "tab.create is rooted in the repo and labelled by its basename" $([ "$(req tab.create)" = "{\"cwd\":\"$S/repo\",\"env\":{\"AGENTBOARD_ACTOR\":\"coder\"},\"label\":\"repo\"}" ] && echo 0 || echo 1)
check "the split targets the root pane, to the right, in the repo" $([ "$(req pane.split)" = "{\"cwd\":\"$S/repo\",\"direction\":\"right\",\"env\":{\"AGENTBOARD_ACTOR\":\"reviewer\"},\"target_pane_id\":\"s:p1\"}" ] && echo 0 || echo 1)
starts=$(req agent.start)
check "coder starts claude with its mcp config and a 120s timeout" $(has "$starts" "\"args\":[\"--strict-mcp-config\",\"--mcp-config=$HOME/.claude/coding-mcp.json\"]" && has "$starts" '"name":"coder"' && has "$starts" '"pane_id":"s:p1"' && has "$starts" '"timeout_ms":120000' && echo 0 || echo 1)
check "reviewer starts opencode with --standalone and no mcp config" $(has "$starts" '"kind":"opencode"' && has "$starts" '"args":["--standalone"]' && ! has "$(sed -n 2p <<<"$starts")" mcp-config && echo 0 || echo 1)
r=$(rows)
check "both are registered by name with their kind, not by pane id" $(has "$r" "coder" && has "$r" "claude" && has "$r" "reviewer" && has "$r" "opencode" && ! has "$r" "s:p" && echo 0 || echo 1)
echo "$r"

stub
M up --repo "$S/repo" --coder codex --reviewer codex --role-name-prefix cx- --env AGENTBOARD_ACTOR=wrong --env KEEP=value
starts=$(req agent.start)
check "codex seats select coder and reviewer profiles despite the name prefix" $([ $rc -eq 0 ] && has "$(head -1 <<<"$starts")" '"args":["-p","coder"]' && has "$(tail -1 <<<"$starts")" '"args":["-p","reviewer"]' && echo 0 || echo 1)
check "pane env overrides a shared actor per role and preserves other variables" $(has "$(req tab.create)" '"env":{"AGENTBOARD_ACTOR":"coder","KEEP":"value"}' && has "$(req pane.split)" '"env":{"AGENTBOARD_ACTOR":"reviewer","KEEP":"value"}' && echo 0 || echo 1)
stub
M up --repo "$S/repo" --coder unknown-kind
check "unknown seat argv kind refuses by name before any herdr call" $([ $rc -eq 1 ] && has "$err" 'unknown-kind' && [ -z "$(methods)" ] && echo 0 || echo 1)

stub
M up --repo "$S/repo" --analyzer codex --role-name-prefix tri- --env AGENTBOARD_ACTOR=wrong --env KEEP=value
check "three-seat up creates three distinct panes in the same tab" $([ $rc -eq 0 ] && has "$out" "tri-analyzer pane s:p3 (codex)" && [ "$(methods)" = "agent.list tab.create pane.split pane.split agent.start agent.list agent.start agent.list agent.start agent.list" ] && echo 0 || echo 1)
check "analyzer reuses its profile argv and per-pane actor environment" $(has "$(req agent.start | tail -1)" '"args":["-p","analyzer"]' && has "$(req pane.split | tail -1)" '"env":{"AGENTBOARD_ACTOR":"analyzer","KEEP":"value"}' && has "$(req pane.split | tail -1)" '"target_pane_id":"s:p2"' && echo 0 || echo 1)
r=$(rows | awk '$1 ~ /^tri-/')
check "three sessions register by role name and kind" $([ "$(wc -l <<<"$r")" = 3 ] && has "$r" tri-coder && has "$r" tri-reviewer && has "$r" tri-analyzer && echo 0 || echo 1)
M status --json
check "manager status joins all three registered seats" $(python3 -c 'import json,sys;r=[x for x in json.loads(sys.argv[1])["agents"] if x["name"].startswith("tri-")];assert len(r)==3;assert {x["name"] for x in r}=={"tri-coder","tri-reviewer","tri-analyzer"};assert all(x["board"]=="registered" for x in r)' "$out" && echo 0 || echo 1)

stub
mv "$S/bin/codex" "$S/bin/codex.saved"
PATH="$S/bin" M up --repo "$S/repo" --analyzer codex --role-name-prefix missing-
echo "missing binary: $err"
check "missing analyzer binary refuses by exact name before any pane exists" $([ $rc -eq 1 ] && [ "$err" = 'agentboard: up: analyzer: binary "codex" is not executable on PATH' ] && [ -z "$(methods)" ] && ! has "$(rows)" missing- && echo 0 || echo 1)
M up --repo "$S/repo" --analyzer codex --env "PATH=$S/bin"
check "binary preflight checks the requested pane PATH" $([ $rc -eq 1 ] && has "$err" 'binary "codex" is not executable on PATH' && [ -z "$(methods)" ] && echo 0 || echo 1)
mv "$S/bin/codex.saved" "$S/bin/codex"
chmod -x "$S/bin/codex"
PATH="$S/bin" M up --repo "$S/repo" --analyzer codex --role-name-prefix nonexec-
check "a non-executable analyzer binary refuses before creating panes" $([ $rc -eq 1 ] && has "$err" 'binary "codex" is not executable on PATH' && [ -z "$(methods)" ] && ! has "$(rows)" nonexec- && echo 0 || echo 1)
chmod +x "$S/bin/codex"
mv "$HOME/.codex/analyzer.config.toml" "$HOME/.codex/analyzer.saved"
M up --repo "$S/repo" --analyzer codex --role-name-prefix profile-
echo "missing profile: $err"
check "missing analyzer profile refuses by exact path before any pane exists" $([ $rc -eq 1 ] && [ "$err" = "agentboard: up: $HOME/.codex/analyzer.config.toml does not exist" ] && [ -z "$(methods)" ] && ! has "$(rows)" profile- && echo 0 || echo 1)
mv "$HOME/.codex/analyzer.saved" "$HOME/.codex/analyzer.config.toml"
mkdir "$S/custom-codex"
CODEX_HOME="$S/custom-codex" M up --repo "$S/repo" --analyzer codex
check "Codex profile preflight respects CODEX_HOME without home fallback" $([ $rc -eq 1 ] && [ "$err" = "agentboard: up: $S/custom-codex/analyzer.config.toml does not exist" ] && [ -z "$(methods)" ] && echo 0 || echo 1)
M up --repo "$S/repo" --analyzer codex --env "CODEX_HOME=$S/custom-codex"
check "profile preflight checks the requested pane CODEX_HOME" $([ $rc -eq 1 ] && [ "$err" = "agentboard: up: $S/custom-codex/analyzer.config.toml does not exist" ] && [ -z "$(methods)" ] && echo 0 || echo 1)
M up --repo "$S/repo" --analyzer claude
echo "missing Claude profile: $err"
check "missing Claude analyzer config refuses before any pane exists" $([ $rc -eq 1 ] && [ "$err" = "agentboard: up: $HOME/.claude/analyzer-mcp.json does not exist" ] && [ -z "$(methods)" ] && echo 0 || echo 1)

export STUB_SEAT=stuck STUB_SEAT_ONLY=analyzer AGENTBOARD_UP_PROMPT_TIMEOUT_MS=2500; stub
M up --repo "$S/repo" --analyzer codex --role-name-prefix fail3-
check "third-seat startup failure closes all three panes and registers no partial team" $([ $rc -eq 1 ] && has "$err" 'fail3-analyzer is not at a prompt' && [ "$(methods | tr ' ' '\n' | grep -c '^pane.close$')" = 3 ] && ! has "$(rows)" fail3- && echo 0 || echo 1)
unset STUB_SEAT STUB_SEAT_ONLY AGENTBOARD_UP_PROMPT_TIMEOUT_MS

stub
M up --repo "$S/repo" --analyzer -unknown-kind
check "hyphen-leading analyzer kind reaches the named store refusal" $([ $rc -eq 1 ] && [ "$err" = 'agentboard: up: no seat argv for kind "-unknown-kind"' ] && [ -z "$(methods)" ] && echo 0 || echo 1)

# A valid first value and unusable final value must never diverge at startup.
mkdir "$S/empty-path" "$S/repeated-codex"
stub
M up --repo "$S/repo" --analyzer codex --role-name-prefix dup-path- --env "PATH=$S/bin" --env "PATH=$S/empty-path"
echo "repeated PATH: $err; pane_calls=$(methods | tr ' ' '\n' | grep -cE '^(tab.create|pane.split)$')"
check "repeated PATH refuses by name with zero pane calls" $([ $rc -eq 1 ] && [ "$err" = 'agentboard: --env duplicate key "PATH"' ] && [ -z "$(methods)" ] && ! has "$(rows)" dup-path- && echo 0 || echo 1)
stub
M up --repo "$S/repo" --analyzer codex --role-name-prefix dup-home- --env "CODEX_HOME=$HOME/.codex" --env "CODEX_HOME=$S/repeated-codex"
echo "repeated CODEX_HOME: $err; pane_calls=$(methods | tr ' ' '\n' | grep -cE '^(tab.create|pane.split)$')"
check "repeated CODEX_HOME refuses by name with zero pane calls" $([ $rc -eq 1 ] && [ "$err" = 'agentboard: --env duplicate key "CODEX_HOME"' ] && [ -z "$(methods)" ] && ! has "$(rows)" dup-home- && echo 0 || echo 1)

export STUB_BUSY=2; stub
M up --repo "$S/repo" --role-name-prefix b-
check "up retries agent_pane_busy while the fresh shell starts, then registers" $([ $rc -eq 0 ] && [ "$(methods | tr ' ' '\n' | grep -c '^agent.start$')" = 4 ] && echo 0 || echo 1)
$B manager down --names b-coder,b-reviewer >/dev/null 2>&1
export STUB_BUSY=1000; stub
M up --repo "$S/repo" --role-name-prefix c-
check "a pane that never becomes a shell fails up, closes only its own two panes, registers nothing" $([ $rc -eq 1 ] && has "$err" agent_pane_busy && has "$err" "closed s:p1, s:p2" && [ "$(methods | tr ' ' '\n' | grep -c '^pane.close$')" = 2 ] && ! has "$(rows)" c-coder && echo 0 || echo 1)
unset STUB_BUSY
export STUB_SEAT=trust; stub
M up --repo "$S/repo" --role-name-prefix t-
echo "up (trust case): $out"
check "up answers a dialog that names --repo, with the exact send_keys params" $([ $rc -eq 0 ] && [ "$(req agent.send_keys | head -1)" = '{"keys":["down","enter"],"target":"s:p1"}' ] && has "$(rows)" t-coder && echo 0 || echo 1)
check "up's one line says it answered the trust dialog for the repo, with the keys" $([ "$(grep -c . <<<"$out")" = 1 ] && has "$out" "t-coder: answered the folder-trust dialog for $S/repo: down, enter" && echo 0 || echo 1)
export STUB_SEAT=trust-other; stub
M up --repo "$S/repo" --role-name-prefix o-
check "a dialog naming another folder gets no keys, fails at once naming the screen, closes its panes, registers nothing" $([ $rc -eq 1 ] && has "$err" "does not show $S/repo" && has "$err" "sent no keys" && has "$err" "/some/other/dir" && ! has "$(methods)" agent.send_keys && has "$err" "closed s:p1, s:p2" && ! has "$(rows)" o-coder && echo 0 || echo 1)
export STUB_SEAT=trust-blank; stub
M up --repo "$S/repo" --role-name-prefix k-
check "a dialog with no readable folder gets no keys either" $([ $rc -eq 1 ] && has "$err" "sent no keys" && ! has "$(methods)" agent.send_keys && ! has "$(rows)" k-coder && echo 0 || echo 1)
export STUB_SEAT=stuck AGENTBOARD_UP_PROMPT_TIMEOUT_MS=2500; stub
M up --repo "$S/repo" --role-name-prefix u-
check "up refuses a seat stuck behind a screen: names status and the screen, closes its panes, registers nothing" $([ $rc -eq 1 ] && has "$err" "u-coder is not at a prompt" && has "$err" "status=blocked launch_pending=true" && has "$err" "Pick a theme" && has "$err" "closed s:p1, s:p2" && ! has "$(rows)" u-coder && echo 0 || echo 1)
export STUB_SEAT=stuck STUB_SEAT_ONLY=reviewer; stub
M up --repo "$S/repo" --role-name-prefix g-
check "the SECOND seat stuck: both panes closed and the first seat's row is gone (no ghost row)" $([ $rc -eq 1 ] && has "$err" "g-reviewer is not at a prompt" && [ "$(methods | tr ' ' '\n' | grep -c '^pane.close$')" = 2 ] && ! has "$(rows)" g-coder && ! has "$(rows)" g-reviewer && echo 0 || echo 1)
unset STUB_SEAT STUB_SEAT_ONLY AGENTBOARD_UP_PROMPT_TIMEOUT_MS
stub '[{"name":"z-coder","agent":"claude","agent_status":"blocked","launch_pending":true,"workspace_id":"s","tab_id":"s:t1","pane_id":"s:p1","focused":false}]'
M down --names z-coder
check "down refuses a launching/blocked seat" $([ $rc -eq 3 ] && has "$err" "refusing to stop z-coder: status=blocked" && echo 0 || echo 1)
M down --names z-coder --force
check "down --force stops a blocked launching seat and says it forced" $([ $rc -eq 0 ] && has "$out" "forced: z-coder was blocked" && has "$(methods)" pane.close && echo 0 || echo 1)
stub '[{"name":"coder","agent":"claude","agent_status":"idle","workspace_id":"s","tab_id":"s:t9","pane_id":"s:p9","focused":false}]'
M up --repo "$S/repo"
check "up refuses a name that is already live and creates nothing" $([ $rc -eq 1 ] && has "$err" "up: coder is already live in s:p9" && [ "$(methods)" = "agent.list" ] && echo 0 || echo 1)
M up --repo "$S/nope"
check "up refuses a repo that is not a directory" $([ $rc -eq 1 ] && has "$err" "not a directory" && echo 0 || echo 1)
rm "$HOME/.claude/reviewer-mcp.json"
M up --repo "$S/repo" --reviewer claude --role-name-prefix x-
check "a missing claude mcp config refuses before any pane is made" $([ $rc -eq 1 ] && has "$err" "reviewer-mcp.json does not exist" && ! has "$(methods)" tab.create && echo 0 || echo 1)
echo '{}' >"$HOME/.claude/reviewer-mcp.json"

W='[{"name":"coder","agent":"claude","agent_status":"working","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p1","focused":false},{"name":"reviewer","agent":"opencode","agent_status":"idle","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p2","focused":false}]'
stub "$W"
M down
echo "down: rc=$rc err=[$err]"
check "down refuses a working coder: exit 3, the exact words" $([ $rc -eq 3 ] && has "$err" "refusing to stop coder: status=working" && echo 0 || echo 1)
check "the refusal closed nothing" $([ "$(methods)" = "agent.list" ] && echo 0 || echo 1)
check "the refusal unregistered nothing" $(has "$(rows)" coder && has "$(rows)" reviewer && echo 0 || echo 1)
: >"$S/log"
M down --force
echo "down --force: $out"
check "down --force closes both panes and says it forced" $([ $rc -eq 0 ] && has "$out" "forced: coder was working" && [ "$(methods)" = "agent.list pane.close pane.close" ] && echo 0 || echo 1)
check "the close requests name the right panes" $([ "$(req pane.close | tr '\n' ' ')" = '{"pane_id":"s:p1"} {"pane_id":"s:p2"} ' ] && echo 0 || echo 1)
check "down --force unregistered both rows" $(rows | grep -qE '^(coder|reviewer) ' && echo 1 || echo 0)
M down
check "down with nothing live and nothing registered is an error, not a silent success" $([ $rc -eq 1 ] && has "$err" "no live agent or board row" && echo 0 || echo 1)

stub '[{"name":"coder","agent":"claude","agent_status":"idle","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p1","focused":false}]'
$B register coder --agent claude >/dev/null
M down --names coder
check "down on an idle agent closes and unregisters without a force note" $([ $rc -eq 0 ] && has "$out" "stopped coder pane s:p1 (was idle); unregistered coder" && ! has "$out" forced && echo 0 || echo 1)

stub "$W"
M renew
check "renew refuses while the coder is working and types nothing" $([ $rc -eq 3 ] && has "$err" "refusing to clear coder: status=working" && [ "$(methods)" = "agent.list" ] && echo 0 || echo 1)
stub '[{"name":"coder","agent":"claude","agent_status":"idle","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p1","focused":false},{"name":"reviewer","agent":"opencode","agent_status":"idle","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p2","focused":false}]'
M renew
echo "renew: $out"
check "renew clears both panes, waits for idle, and cites a prompt marker from each screen" $([ $rc -eq 0 ] && [ "$(methods)" = "agent.list pane.send_text pane.send_text pane.send_keys pane.send_keys agent.wait pane.read agent.wait pane.read" ] || [ "$(methods)" = "agent.list pane.send_text pane.send_keys pane.send_text pane.send_keys agent.wait pane.read agent.wait pane.read" ] && has "$out" "renewed coder pane s:p1: idle, prompt marker" && has "$out" "renewed reviewer pane s:p2: idle, prompt marker" && echo 0 || echo 1)
export STUB_PROMPT=none; stub '[{"name":"coder","agent":"claude","agent_status":"idle","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p1","focused":false}]'
M renew --names coder
check "renew refuses an idle pane whose screen shows no prompt marker, and prints the screen" $([ $rc -eq 1 ] && has "$err" "no prompt marker" && has "$err" "auto mode on" && [ -z "$out" ] && echo 0 || echo 1)
unset STUB_PROMPT

# agentboard wait --agent: one call watches board rows and an agent's transitions.
EV='[{"name":"coder","agent":"claude","agent_status":"idle","workspace_id":"s","tab_id":"s:t1","pane_id":"s:p1","focused":false}]'
export STUB_EVENTS_LOG=$S/events.log
T=$($B add "watched row" 2>/dev/null | grep -o 'T[0-9]*' | head -1)
reset_events() { : >"$STUB_EVENTS_LOG"; }
reset_events; export STUB_EVENTS=300:coder:idle; stub "$EV"
run $B wait "$T" --agent coder --status done --timeout 5
echo "wait --agent (agent fires): $out"
check "the agent's transition wakes a wait on a quiet board row, naming the agent side" $([ $rc -eq 0 ] && [ "$out" != "${out#matched: agent coder -> idle after 0.}" ] && echo 0 || echo 1)
check "...and the stream is closed by the client (stub saw EOF)" $(for _ in $(seq 1 20); do grep -q "closed by client" "$STUB_EVENTS_LOG" && break; sleep 0.1; done; grep -q "closed by client" "$STUB_EVENTS_LOG" && echo 0 || echo 1)
unset STUB_EVENTS
reset_events; stub "$EV"
( sleep 0.5; $B claim "$T" >/dev/null 2>&1 ) &
CLAIMER=$!
run $B wait "$T" --agent coder --status working --timeout 5
wait "$CLAIMER"
echo "wait --agent (board fires): $out"
check "a board row moving first wakes it with the board's own text, not an agent line" $([ $rc -eq 0 ] && has "$out" "matched: 1 of 1 watched" && ! has "$out" "agent coder" && echo 0 || echo 1)
check "...and the stream is closed by the client when the board wins" $(for _ in $(seq 1 20); do grep -q "closed by client" "$STUB_EVENTS_LOG" && break; sleep 0.1; done; grep -q "closed by client" "$STUB_EVENTS_LOG" && echo 0 || echo 1)
reset_events; stub "$EV"
run $B wait "$T" --agent coder --status done --timeout 1
echo "wait --agent (timeout): $out"
check "nothing firing is one timeout, exit 3, with the board's text and the agent line" $([ $rc -eq 3 ] && has "$out" "timeout: nothing matched after 1s" && has "$out" "timeout: no agent transition after 1s (watching coder)" && echo 0 || echo 1)
reset_events; export STUB_EVENTS=200:coder:working; stub "$EV"
run $B wait --agent coder --until working --timeout 5
check "--agent alone needs no task ids" $([ $rc -eq 0 ] && has "$out" "matched: agent coder -> working after 0." && echo 0 || echo 1)
unset STUB_EVENTS
run $B wait --agent coder --status done --timeout 1
check "--status without task ids is refused, not silently ignored" $([ $rc -eq 1 ] && has "$err" "need task ids, --epic or --open" && echo 0 || echo 1)
run $B wait "$T" --until idle --timeout 1
check "--until without --agent is refused" $([ $rc -eq 1 ] && has "$err" "--agent" && echo 0 || echo 1)
run $B wait "$T" --agent ghost --timeout 1
check "an unknown agent is refused before blocking" $([ $rc -eq 1 ] && has "$err" 'no live agent "ghost"' && echo 0 || echo 1)
unset STUB_EVENTS_LOG

# A herdr that hangs must not discard a board match: the row is already working.
$B claim "$T" >/dev/null 2>&1
export STUB_HANG=agent.list; stub "$EV"
run env AGENTBOARD_HERDR_TIMEOUT_MS=500 $B wait "$T" --agent coder --status working --timeout 8
echo "wait --agent (herdr hangs, board matched): out=[$out] err=[$err]"
check "a board match is returned with exit 0 though herdr never answers" $([ $rc -eq 0 ] && has "$out" "matched: 1 of 1 watched" && echo 0 || echo 1)
check "...and the degraded --agent side is noted on stderr, not swallowed" $(has "$err" "board matched, --agent watch degraded: wait: agent watch failed: herdr: agent.list timed out" && echo 0 || echo 1)
run env AGENTBOARD_HERDR_TIMEOUT_MS=500 $B wait "$T" --agent coder --status done --timeout 8
check "with no board match the herdr error decides, naming the agent side" $([ $rc -eq 1 ] && [ -z "$out" ] && has "$err" "wait: agent watch failed: herdr: agent.list timed out" && echo 0 || echo 1)
unset STUB_HANG

# Layout CLI contracts; every apply targets the stub explicitly.
stub
M layout export --json
layout=$out
echo "layout export: $layout"
check "layout export passes through herdr's ratio" $([ $rc -eq 0 ] && has "$out" '"ratio":0.37251' && echo 0 || echo 1)
run bash -c 'printf "%s" "$1" | "$2" manager layout apply --tab s:t1 --root -' _ "$layout" "$B"
check "layout apply accepts the exported tree on stdin" $([ $rc -eq 0 ] && has "$out" "applied layout to s:t1" && echo 0 || echo 1)
check "layout apply sends the exported root and explicit tab" $(python3 -c 'import json,sys; rows=[json.loads(l) for l in open(sys.argv[1])]; p=rows[-1]["params"]; assert p=={"tab_id":"s:t1","root":json.loads(sys.argv[2])}' "$S/log" "$layout" && echo 0 || echo 1)
stub
M layout apply --tab s:t1
check "layout apply without root is one line, exit 1, nothing sent" $([ $rc -eq 1 ] && has "$err" "requires --root" && [ "$(wc -l < "$S/err")" = 1 ] && [ -z "$(methods)" ] && echo 0 || echo 1)
M layout apply --root "$layout"
check "layout apply without tab refuses instead of guessing" $([ $rc -eq 1 ] && has "$err" "requires --tab" && [ -z "$(methods)" ] && echo 0 || echo 1)
M layout apply --tab s:t1 --root '{'
check "layout apply invalid JSON is one line and sends nothing" $([ $rc -eq 1 ] && has "$err" "invalid JSON" && [ "$(wc -l < "$S/err")" = 1 ] && [ -z "$(methods)" ] && echo 0 || echo 1)
M layout apply --tab s:t1 --root '{}'
check "layout apply empty tree is one line and sends nothing" $([ $rc -eq 1 ] && has "$err" "malformed or empty tree" && [ "$(wc -l < "$S/err")" = 1 ] && [ -z "$(methods)" ] && echo 0 || echo 1)

echo "$pass/$total passed (backend: $BACKEND)"
[ "$pass" -eq "$total" ]
