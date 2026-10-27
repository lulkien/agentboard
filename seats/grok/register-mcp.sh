#!/bin/sh
# Compatibility wrapper for the managed Grok MCP entry in seats/install.py.
# usage: register-mcp.sh [--actor NAME]
set -eu
exec python3 "$(dirname "$0")/../install.py" --grok-only "$@"
