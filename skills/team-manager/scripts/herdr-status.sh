#!/usr/bin/env bash
# Read-only herdr status dump: server, tabs, panes, agents, board sessions.
# Safe to run any time; changes nothing.
set -u

# Two ways to be "inside herdr": the pane exports HERDR_ENV, or the CLI can reach
# a running herdr server. Hermes' own tool shell is a herdr pane without the env
# var, so the env check alone would wrongly refuse here.
if [ "${HERDR_ENV:-}" != 1 ] && ! herdr status >/dev/null 2>&1; then
  echo "NOT_INSIDE_HERDR — herdr commands must run from a pane inside herdr" >&2
  exit 1
fi

echo "== server =="
herdr status 2>&1 | sed 's/^/  /'

echo "== tabs =="
herdr tab list 2>/dev/null | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("  (unparsed)"); raise SystemExit
for t in d["result"]["tabs"]:
    print("  {:<7} panes={} focused={} label={!r}".format(
        t["tab_id"], t["pane_count"], t["focused"], t.get("label")))
'

echo "== panes =="
herdr pane list 2>/dev/null | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("  (unparsed)"); raise SystemExit
for p in d["result"]["panes"]:
    print("  {:<7} tab={:<6} agent={:<9} status={:<8} {}".format(
        p["pane_id"], p["tab_id"], str(p.get("agent")), str(p.get("agent_status")),
        p.get("foreground_cwd")))
'

echo "== agents =="
herdr agent list 2>/dev/null | python3 -c '
import json, sys
try:
    d = json.load(sys.stdin)
except Exception:
    print("  (unparsed)"); raise SystemExit
agents = d["result"]["agents"]
if not agents:
    print("  (none)")
for a in agents:
    print("  {:<10} {:<9} {:<8} {:<7} {}".format(
        str(a.get("name")), a["agent"], str(a.get("agent_status")), a["pane_id"], a["cwd"]))
'

echo "== board sessions =="
if command -v agentboard >/dev/null 2>&1; then
  agentboard sessions 2>&1 | sed 's/^/  /'
else
  echo "  agentboard not installed"
fi
