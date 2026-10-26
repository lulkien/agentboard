#!/usr/bin/env bash
set -eu
cd "$(dirname "$0")/.."
repo=$(pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
home=$tmp/home
mkdir -p "$home/.claude" "$home/.config/opencode" "$home/.codex/rules"
for seat in analyst coder reviewer; do
  ln -s "/old/checkout/seats/claude/$seat-mcp.json" "$home/.claude/$seat-mcp.json"
done
ln -s /old/checkout/seats/opencode/agentboard-mcp.sh "$home/.config/opencode/agentboard-mcp.sh"

if HOME="$home" CODEX_HOME="$home/.codex" bash ./seats/install.sh >"$tmp/out" 2>"$tmp/err"; then
  echo "expected foreign seat links to be refused without --force" >&2
  exit 1
fi
if ! grep -q 'use --force to replace it' "$tmp/err"; then
  cat "$tmp/err" >&2
  exit 1
fi
test ! -s "$tmp/out"

HOME="$home" CODEX_HOME="$home/.codex" bash ./seats/install.sh --force >"$tmp/out" 2>"$tmp/err"
for seat in analyst coder reviewer; do
  test "$(readlink "$home/.claude/$seat-mcp.json")" = "$repo/seats/claude/$seat-mcp.json"
done
test "$(readlink "$home/.config/opencode/agentboard-mcp.sh")" = "$repo/seats/opencode/agentboard-mcp.sh"
echo '4/4 passed (seats installer foreign links)'
