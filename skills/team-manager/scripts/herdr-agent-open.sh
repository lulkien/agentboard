#!/usr/bin/env bash
# Open a project-scoped herdr agent: close any live agent with the same name,
# close its pane, create a pane rooted in the project, start the agent there.
#
# Usage: herdr-agent-open <name> <kind> <repo-path> [model]
#   name: coder | reviewer (reviewer gets reviewer-mcp.json so its board actor differs)
set -u
name=${1:?usage: herdr-agent-open <name> <kind> <repo> [model]}
kind=${2:?usage: herdr-agent-open <name> <kind> <repo> [model]}
repo=${3:?usage: herdr-agent-open <name> <kind> <repo> [model]}
model=${4:-sonnet}

[ "${HERDR_ENV:-}" = 1 ] || herdr status >/dev/null 2>&1 || { echo "not inside herdr (HERDR_ENV != 1, no herdr server reachable)" >&2; exit 1; }
[ -d "$repo" ] || { echo "no such repo: $repo" >&2; exit 1; }

# 1. Close a running agent of that name: send-keys cannot quit a TUI, so kill
#    the pane's foreground process group, then close the pane itself.
if herdr agent list | rg -q "\"name\":\"$name\""; then
  pane=$(herdr agent list | python3 -c "import json,sys; d=json.load(sys.stdin); print(next(a['pane_id'] for a in d['result']['agents'] if a.get('name')=='$name'))")
  # Never stop a working agent. A pane is closed only on an explicit stop, so
  # switching project waits for the current task to settle unless FORCE=1.
  st=$(herdr agent list | python3 -c "import json,sys; d=json.load(sys.stdin); print(next((a.get('agent_status') or 'unknown') for a in d['result']['agents'] if a.get('name')=='$name'))")
  case "$st" in
    working|blocked)
      if [ "${FORCE:-0}" != 1 ]; then
        echo "refusing to stop $name: status=$st; wait for it to settle or pass FORCE=1" >&2
        exit 3
      fi
      ;;
  esac
  pgid=$(herdr pane process-info --pane "$pane" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["process_info"]["foreground_process_group_id"])')
  kill -TERM "-$pgid" 2>/dev/null
  sleep 2
  herdr pane close "$pane" >/dev/null 2>&1
  echo "closed agent $name and pane $pane (was $st)"
fi

# 2. New pane rooted in the project; focus stays where it is.
pane=$(herdr pane split --current --direction down --cwd "$repo" --no-focus \
        | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')
sleep 1

# 3. Start the agent with the config that matches its role.
if [ "$kind" = claude ]; then
  conf=$HOME/.claude/coding-mcp.json
  [ "$name" = reviewer ] && conf=$HOME/.claude/reviewer-mcp.json
  # Optional voice/compression prompt: pass it only when the file is present, or
  # claude refuses to start on a machine that does not carry it.
  voice=()
  [ -f "$HOME/.claude/caveman-full.md" ] && \
    voice=(--append-system-prompt-file="$HOME/.claude/caveman-full.md")
  herdr agent start "$name" --kind claude --pane "$pane" --timeout 120000 -- \
    --strict-mcp-config --mcp-config="$conf" ${voice[@]+"${voice[@]}"} --model "$model" >/dev/null 2>&1
elif [ "$kind" = opencode ]; then
  # Model comes from opencode's config (top-level "model"); the TUI entry rejects
  # --model. --standalone keeps the TUI up: against a broken shared background
  # service opencode prints its banner and exits, and the start times out.
  herdr agent start "$name" --kind opencode --pane "$pane" --timeout 120000 -- --standalone >/dev/null 2>&1
else
  herdr agent start "$name" --kind "$kind" --pane "$pane" --timeout 120000 >/dev/null 2>&1
fi

# 4. herdr may name the agent after its kind when a startup dialog delayed
#    readiness; force the requested name.
herdr agent list | grep -q "\"name\":\"$name\"" || herdr agent rename "$pane" "$name" >/dev/null 2>&1

# 5. A fresh cwd triggers the folder-trust dialog; answer it before reporting.
herdr pane read "$pane" --lines 6 2>/dev/null | grep -qi 'trust this folder' && \
  herdr agent send-keys "$pane" down enter

echo "agent $name ($kind, $model) in $pane, cwd $repo"
herdr agent list | python3 -c "import json,sys; d=json.load(sys.stdin); [print(a.get('name'), a['agent'], a.get('agent_status'), a['cwd']) for a in d['result']['agents']]"
