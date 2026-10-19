#!/bin/sh
# Install every skills/<name>/ directory into a skills dir.
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

# Check every destination first, so a refusal touches nothing.
for dir in "$src"/*/; do
  name=$(basename "$dir")
  if [ -e "$target/$name" ] && [ "$force" -eq 0 ]; then
    echo "exists: $target/$name (use --force)" >&2
    exit 1
  fi
done

mkdir -p "$target"
for dir in "$src"/*/; do
  name=$(basename "$dir")
  rm -rf "${target:?}/$name"
  cp -R "$dir" "$target/$name"
  echo "installed: $name -> $target/$name"
done
