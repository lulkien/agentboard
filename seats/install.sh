#!/bin/sh
# Link the seat cards into their client homes: one physical copy, in this repo.
# usage: install.sh [--force]
set -eu
force=0
for arg in "$@"; do
  case $arg in
    --force) force=1 ;;
    *) echo "usage: install.sh [--force]" >&2; exit 2 ;;
  esac
done
src=$(cd "$(dirname "$0")" && pwd)
codex_home=${CODEX_HOME:-$HOME/.codex}
claude_home=$HOME/.claude
opencode_home=$HOME/.config/opencode
repo=$(cd "$src/.." && pwd)

refuse=0
check() {
  if [ -L "$1" ]; then
    case $(readlink "$1") in "$repo"/*) ;; *) echo "foreign link: $1 -> $(readlink "$1") (move it aside)" >&2; refuse=1 ;; esac
  elif [ -e "$1" ] && [ "$force" -eq 0 ]; then
    echo "exists: $1 (use --force)" >&2
    refuse=1
  fi
}
for seat in analyzer coder manager reviewer; do check "$codex_home/$seat.config.toml"; done
for seat in analyzer coder reviewer; do check "$claude_home/$seat-mcp.json"; done
check "$opencode_home/agentboard-mcp.sh"
[ "$refuse" -eq 0 ] || exit 1

link() {
  mkdir -p "$(dirname "$2")"
  rm -f "${2:?}"
  ln -s "$1" "$2"
  echo "linked: $2 -> $1"
}
for seat in analyzer coder manager reviewer; do
  link "$src/codex/$seat.config.toml" "$codex_home/$seat.config.toml"
done
for seat in analyzer coder reviewer; do
  link "$src/claude/$seat-mcp.json" "$claude_home/$seat-mcp.json"
done
link "$src/opencode/agentboard-mcp.sh" "$opencode_home/agentboard-mcp.sh"

if [ -e "$codex_home/config.toml" ] && ! grep -q 'agentboard' "$codex_home/config.toml"; then
  echo "note: merge $src/codex/base.config.toml into $codex_home/config.toml" >&2
fi
echo "done: the codex base config is merged by hand; see seats/README.md"
