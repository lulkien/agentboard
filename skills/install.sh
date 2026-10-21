#!/bin/sh
# Link every skills/<name>/ into a skills dir and the manager slash commands into both CLIs.
# One file, not a copy: the installed path IS the repo file, so there is no drift to prove.
# usage: install.sh [--force] [target-dir]   (default target: $HOME/.hermes/skills)
set -eu

force=0
target=
for arg in "$@"; do
  case $arg in
    --force) force=1 ;;
    *) target=$arg ;;
  esac
done
target=${target:-$HOME/.hermes/skills}
src=$(cd "$(dirname "$0")" && pwd)
cmd_src=$src/agentboard-manager/commands
claude_dir=${AGENTBOARD_CLAUDE_COMMANDS:-$HOME/.claude/commands}
opencode_dir=${AGENTBOARD_OPENCODE_COMMANDS:-$HOME/.config/opencode/commands}

# Check every destination first, so a refusal touches nothing.
refuse=0
for dir in "$src"/*/; do
  p=$target/$(basename "$dir")
  if { [ -e "$p" ] || [ -L "$p" ]; } && [ "$force" -eq 0 ]; then
    echo "exists: $p (use --force)" >&2; refuse=1
  fi
done
for p in "$claude_dir/manager.md" "$opencode_dir/manager.md"; do
  if { [ -e "$p" ] || [ -L "$p" ]; } && [ "$force" -eq 0 ]; then
    echo "exists: $p (use --force)" >&2; refuse=1
  fi
done
[ "$refuse" -eq 0 ] || exit 1

link() { # link <repo path> <dest>
  rm -rf "${2:?}"
  ln -s "$1" "$2"
  echo "linked: $2 -> $1"
}

mkdir -p "$target" "$claude_dir" "$opencode_dir"
for dir in "$src"/*/; do
  link "${dir%/}" "$target/$(basename "$dir")"
done
link "$cmd_src/claude/manager.md" "$claude_dir/manager.md"
link "$cmd_src/opencode/manager.md" "$opencode_dir/manager.md"
