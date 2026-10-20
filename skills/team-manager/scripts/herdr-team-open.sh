#!/usr/bin/env bash
# Open a team tab in herdr: one new tab, two panes, coder left, reviewer right.
# The manager keeps its own tab. Closes any live coder/reviewer agent first.
#
# Usage: herdr-team-open <repo-path>
#   REVIEWER_KIND=claude     reviewer runs Claude instead of opencode (default opencode)
#   REVIEWER_MODEL=opus      model for a Claude reviewer (default sonnet)
#   CODER_MODEL=sonnet       model for the coder
#   DRY=1                    print the plan, change nothing
set -u

repo=${1:?usage: herdr-team-open <repo>}
[ -d "$repo" ] || { echo "no such repo: $repo" >&2; exit 1; }
if [ "${HERDR_ENV:-}" != 1 ]; then
  echo "not inside herdr (HERDR_ENV != 1); start herdr first" >&2
  exit 1
fi

REVIEWER_KIND=${REVIEWER_KIND:-opencode}
REVIEWER_MODEL=${REVIEWER_MODEL:-sonnet}
CODER_MODEL=${CODER_MODEL:-sonnet}
DRY=${DRY:-0}

say() { printf '%s\n' "$*"; }
run() { if [ "$DRY" = 1 ]; then printf '[dry-run] %s\n' "$*"; else "$@"; fi; }

json_get() { python3 -c "import json,sys;d=json.load(sys.stdin);print($1)" 2>/dev/null; }

close_agent() {
  local name=$1 pane pgid
  herdr agent list | grep -q "\"name\":\"$name\"" || return 0
  pane=$(herdr agent list | json_get "next(a['pane_id'] for a in d['result']['agents'] if a.get('name')=='$name')")
  [ -n "$pane" ] || return 0
  pgid=$(herdr pane process-info --pane "$pane" | json_get "d['result']['process_info']['foreground_process_group_id']")
  run kill -TERM "-$pgid"
  [ "$DRY" = 1 ] || sleep 2
  run herdr pane close "$pane"
  say "closed $name (pane $pane)"
}

# 1. free the names and their panes
close_agent coder
close_agent reviewer

# 2. one tab, rooted in the repo; its root pane hosts the coder
if [ "$DRY" = 1 ]; then
  say "[dry-run] herdr tab create --cwd $repo --label $(basename "$repo")"
  coder_pane='<coder-pane>'; reviewer_pane='<reviewer-pane>'
else
  created=$(herdr tab create --cwd "$repo" --label "$(basename "$repo")") || {
    echo "tab create failed: $created" >&2; exit 1; }
  coder_pane=$(printf '%s' "$created" | json_get "d['result']['root_pane']['pane_id']")
  reviewer_pane=$(herdr pane split --pane "$coder_pane" --direction right --cwd "$repo" --no-focus \
                  | json_get "d['result']['pane']['pane_id']")
  sleep 1
  say "team tab for $repo: coder=$coder_pane reviewer=$reviewer_pane"
fi

start_agent() {
  local name=$1 pane=$2 kind=$3 model=$4 conf
  case "$kind" in
    claude)
      conf=$HOME/.claude/coding-mcp.json
      [ "$name" = reviewer ] && conf=$HOME/.claude/reviewer-mcp.json
      run herdr agent start "$name" --kind claude --pane "$pane" --timeout 120000 -- \
        --strict-mcp-config --mcp-config="$conf" --model "$model"
      ;;
    opencode|codex|gemini)
      run herdr agent start "$name" --kind "$kind" --pane "$pane" --timeout 120000
      ;;
    *) echo "unsupported kind: $kind" >&2; return 1 ;;
  esac
  [ "$DRY" = 1 ] && return 0
  # herdr names an agent after its kind when a startup dialog delayed readiness
  herdr agent list | grep -q "\"name\":\"$name\"" || herdr agent rename "$pane" "$name" >/dev/null 2>&1
  # folder-trust dialog on first start in a repo: cursor starts on "No, exit"
  herdr pane read "$pane" --lines 8 2>/dev/null | grep -qi 'trust this folder' && \
    herdr agent send-keys "$pane" down enter
  return 0
}

start_agent coder "$coder_pane" claude "$CODER_MODEL"
start_agent reviewer "$reviewer_pane" "$REVIEWER_KIND" "$REVIEWER_MODEL"

[ "$DRY" = 1 ] && { say "[dry-run] no agents started"; exit 0; }

sleep 2
herdr agent list | python3 -c '
import json,sys
for a in json.load(sys.stdin)["result"]["agents"]:
    print(a.get("name"), a["agent"], a.get("agent_status"), a["cwd"], a["pane_id"])'
say ""
say "Register on the board:"
say "  agentboard register $coder_pane --agent claude --task T<id>"
say "  agentboard register $reviewer_pane --agent claude --task T<id>"
