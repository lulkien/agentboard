#!/bin/sh
# Canonical repo links in ~/.agents/skills, then agent aliases through that home.
# Every skill directory that carries a claude.md or an opencode.md also gets that
# CLI's command link, so a seat entry point is one edit in the repo. Links under a
# retired name are pruned when they are exactly the link this installer wrote.
# usage: install.sh [--force] [canonical-dir]
set -eu
force=0
target=
for arg in "$@"; do
  case $arg in
    --force) force=1 ;;
    *) target=$arg ;;
  esac
done
target=${target:-$HOME/.agents/skills}
case $target in /*) ;; *) target=$PWD/$target ;; esac
src=$(cd "$(dirname "$0")" && pwd)
claude_skills=$HOME/.claude/skills
codex_skills=${CODEX_HOME:-$HOME/.codex}/skills
opencode_skills=$HOME/.config/opencode/skills
hermes_skills=$HOME/.hermes/skills
claude_commands=${AGENTBOARD_CLAUDE_COMMANDS:-$HOME/.claude/commands}
opencode_commands=${AGENTBOARD_OPENCODE_COMMANDS:-$HOME/.config/opencode/commands}

# Enumerate the entire write set before touching any parent or destination.
refuse=0
check() {
  if { [ -e "$1" ] || [ -L "$1" ]; } && [ "$force" -eq 0 ]; then
    echo "exists: $1 (use --force)" >&2
    refuse=1
  fi
}
for dir in "$src"/*/; do
  [ -f "$dir/SKILL.md" ] || continue
  name=$(basename "$dir")
  check "$target/$name"
  for agent_dir in "$claude_skills" "$codex_skills" "$opencode_skills" "$hermes_skills"; do
    # A caller may explicitly choose an agent's directory as the canonical home.
    [ "$agent_dir" = "$target" ] || check "$agent_dir/$name"
  done
  if [ -f "$dir/claude.md" ]; then check "$claude_commands/$name.md"; fi
  if [ -f "$dir/opencode.md" ]; then check "$opencode_commands/$name.md"; fi
done
[ "$refuse" -eq 0 ] || exit 1

# Retired skill names. A link is removed only when it points exactly where this
# installer pointed it; a foreign link, a real file or a real directory stays.
retired="manager analyst coder reviewer grok-manager analyzer"
prune() {
  if [ -L "$1" ] && [ "$(readlink "$1")" = "$2" ]; then
    rm -f "$1"
    echo "pruned: $1"
  fi
}
for name in $retired; do
  [ -d "$src/$name" ] && continue
  prune "$target/$name" "$src/$name"
  for agent_dir in "$claude_skills" "$codex_skills" "$opencode_skills" "$hermes_skills"; do
    [ "$agent_dir" = "$target" ] || prune "$agent_dir/$name" "$target/$name"
  done
  prune "$claude_commands/$name.md" "$target/$name/claude.md"
  prune "$opencode_commands/$name.md" "$target/$name/opencode.md"
done

link() {
  mkdir -p "$(dirname "$2")"
  rm -rf "${2:?}"
  ln -s "$1" "$2"
  echo "linked: $2 -> $1"
}
for dir in "$src"/*/; do
  [ -f "$dir/SKILL.md" ] || continue
  name=$(basename "$dir")
  link "${dir%/}" "$target/$name"
  for agent_dir in "$claude_skills" "$codex_skills" "$opencode_skills" "$hermes_skills"; do
    [ "$agent_dir" = "$target" ] || link "$target/$name" "$agent_dir/$name"
  done
  if [ -f "$dir/claude.md" ]; then link "$target/$name/claude.md" "$claude_commands/$name.md"; fi
  if [ -f "$dir/opencode.md" ]; then link "$target/$name/opencode.md" "$opencode_commands/$name.md"; fi
done
