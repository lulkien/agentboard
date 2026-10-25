#!/bin/sh
# Register the agentboard MCP server for a Grok Build seat.
# One actor per registration: Grok keeps one user config in ~/.grok/config.toml.
# usage: register-mcp.sh [--actor NAME]
set -eu
actor=grok
while [ $# -gt 0 ]; do
  case $1 in
    --actor) [ $# -ge 2 ] || { echo "usage: register-mcp.sh [--actor NAME]" >&2; exit 2; }
             actor=$2; shift 2 ;;
    *) echo "usage: register-mcp.sh [--actor NAME]" >&2; exit 2 ;;
  esac
done
command -v grok >/dev/null 2>&1 || { echo "grok is not on PATH" >&2; exit 1; }
bin=$(command -v agentboard 2>/dev/null || true)
[ -n "$bin" ] || bin=$HOME/.cargo/bin/agentboard
[ -x "$bin" ] || { echo "agentboard is not installed (looked on PATH and in ~/.cargo/bin)" >&2; exit 1; }
grok mcp add --scope user -e "AGENTBOARD_ACTOR=$actor" agentboard -- "$bin" serve
echo "registered: agentboard -> $bin serve (actor $actor)"
grok mcp list
