#!/bin/sh
# Link the seat cards into their client homes: one physical copy, in this repo.
# The Codex rules are not a link: they are a marked block inside the user's own
# rules/default.rules, which Codex also appends to.
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

rules_src=$src/codex/rules/agentboard.rules
rules_dir=$codex_home/rules
rules_file=$rules_dir/default.rules
rules_begin='# BEGIN AGENTBOARD RULES'
rules_end='# END AGENTBOARD RULES'
rules_tmp=
trap '[ -z "$rules_tmp" ] || rm -f "$rules_tmp"' EXIT

# rules_scan <file>: prints "none", "ok BEGIN-LINE END-LINE" or "bad LINES".
# A marker is a whole line, ignoring one trailing CR; a line that matches only
# after trimming whitespace counts as a broken marker, never as a marker.
rules_scan() {
  awk -v begin="$rules_begin" -v end="$rules_end" '
    { line = $0; sub(/\r$/, "", line); t = line; gsub(/^[ \t]+|[ \t]+$/, "", t) }
    line == begin { nb++; if (!fb) fb = NR; list = list (list == "" ? "" : ",") NR; next }
    line == end { ne++; if (!fe) fe = NR; list = list (list == "" ? "" : ",") NR; next }
    t == begin || t == end { bad++; list = list (list == "" ? "" : ",") NR }
    END {
      if (nb + ne + bad == 0) print "none"
      else if (bad || nb != 1 || ne != 1 || fe < fb) print "bad " list
      else print "ok " fb " " fe
    }' "$1"
}

refuse=0
check() {
  if [ -L "$1" ]; then
    case $(readlink "$1") in
      "$repo"/*) ;;
      *)
        if [ "$force" -eq 0 ]; then
          echo "foreign link: $1 -> $(readlink "$1") (use --force to replace it)" >&2
          refuse=1
        fi
        ;;
    esac
  elif [ -e "$1" ] && [ "$force" -eq 0 ]; then
    echo "exists: $1 (use --force)" >&2
    refuse=1
  fi
}
for seat in analyst coder manager reviewer; do check "$codex_home/$seat.config.toml"; done

# default.rules is judged here, with the other checks, so a refusal links nothing.
rules_source_ok=$(awk -v begin="$rules_begin" -v end="$rules_end" '
  { line = $0; sub(/\r$/, "", line); t = line; gsub(/^[ \t]+|[ \t]+$/, "", t) }
  t == begin || t == end { n++ }
  NR == 1 { first = line }
  { last = line }
  END { print (n == 2 && first == begin && last == end) ? "yes" : "no" }' "$rules_src")
if [ "$rules_source_ok" != yes ]; then
  echo "bad rules source: $rules_src (must start with BEGIN, end with END, and hold no other marker)" >&2
  refuse=1
elif [ -L "$rules_file" ]; then
  echo "$rules_file is a symlink; the installer edits only a regular file" >&2
  refuse=1
elif [ -e "$rules_file" ] && [ ! -f "$rules_file" ]; then
  echo "$rules_file is not a regular file; the installer edits only a regular file" >&2
  refuse=1
elif [ -f "$rules_file" ]; then
  case $(rules_scan "$rules_file") in
    bad*)
      set -- $(rules_scan "$rules_file")
      echo "malformed agentboard block in $rules_file (marker lines $2); fix it by hand" >&2
      refuse=1
      ;;
  esac
fi
for seat in analyst coder manager reviewer; do check "$claude_home/$seat-mcp.json"; done
check "$opencode_home/agentboard-mcp.sh"
[ "$refuse" -eq 0 ] || exit 1

link() {
  mkdir -p "$(dirname "$2")"
  rm -f "${2:?}"
  ln -s "$1" "$2"
  echo "linked: $2 -> $1"
}
# Put the repo's block into default.rules: append when absent, replace in place
# only under --force, never touch anything outside the marker lines.
install_rules_block() {
  mkdir -p "$rules_dir"
  sum_before=
  nl='\n'
  state=missing
  if [ -f "$rules_file" ]; then
    sum_before=$(cksum <"$rules_file")
    state=$(rules_scan "$rules_file")
    if [ "$(head -n 1 "$rules_file" | tail -c 2 | od -An -c | tr -d ' ')" = '\r\n' ]; then
      nl='\r\n'
    fi
  fi
  emit_block() {
    awk -v nl="$nl" '{ sub(/\r$/, ""); printf "%s%s", $0, nl }' "$rules_src"
  }
  case $state in
    ok*)
      set -- $state
      begin_line=$2 end_line=$3
      if [ "$force" -eq 0 ]; then
        echo "unchanged: $rules_file (agentboard block present; --force rewrites it)"
        existing=$(sed -n "${begin_line},${end_line}p" "$rules_file" | tr -d '\r')
        if [ "$existing" != "$(tr -d '\r' <"$rules_src")" ]; then
          echo "note: agentboard block in $rules_file differs from the repo; use --force" >&2
        fi
        return 0
      fi
      verb=replaced
      ;;
    *) verb=added ;;
  esac
  rules_tmp=$rules_dir/.default.rules.agentboard.$$
  if [ -f "$rules_file" ]; then
    cp -p "$rules_file" "$rules_tmp"
    case $state in
      ok*)
        {
          head -n $((begin_line - 1)) "$rules_file"
          emit_block
          tail -n +$((end_line + 1)) "$rules_file"
        } >"$rules_tmp"
        ;;
      *)
        if [ -s "$rules_file" ] && [ "$(tail -c 1 "$rules_file" | wc -l)" -eq 0 ]; then
          printf "$nl" >>"$rules_tmp"
        fi
        emit_block >>"$rules_tmp"
        ;;
    esac
  else
    emit_block >"$rules_tmp"
  fi
  if [ -f "$rules_file" ] && cmp -s "$rules_tmp" "$rules_file"; then
    rm -f "$rules_tmp"
    rules_tmp=
    echo "unchanged: $rules_file (agentboard block already current)"
    return 0
  fi
  if [ -f "$rules_file" ] && [ "$(cksum <"$rules_file")" != "$sum_before" ]; then
    echo "$rules_file changed during install; rerun with no Codex session open" >&2
    exit 1
  fi
  mv -f "$rules_tmp" "$rules_file"
  rules_tmp=
  echo "$verb: $rules_file (agentboard block)"
}

# The old install linked rules/agentboard.rules; Codex would load it as well.
remove_legacy_rules_link() {
  legacy=$rules_dir/agentboard.rules
  if [ -L "$legacy" ]; then
    case $(readlink "$legacy") in
      "$repo"/*)
        rm -f "$legacy"
        echo "removed: $legacy (now a block in default.rules)"
        ;;
      *)
        if [ "$force" -eq 1 ]; then
          rm -f "$legacy"
          echo "removed: $legacy (foreign link)"
        else
          echo "note: foreign link $legacy; --force removes it" >&2
        fi
        ;;
    esac
  elif [ -e "$legacy" ]; then
    echo "note: $legacy is a regular file; agentboard rules may load twice" >&2
  fi
}

for seat in analyst coder manager reviewer; do
  link "$src/codex/$seat.config.toml" "$codex_home/$seat.config.toml"
done
install_rules_block
remove_legacy_rules_link
for seat in analyst coder manager reviewer; do
  link "$src/claude/$seat-mcp.json" "$claude_home/$seat-mcp.json"
done
link "$src/opencode/agentboard-mcp.sh" "$opencode_home/agentboard-mcp.sh"

if [ -e "$codex_home/config.toml" ] && ! grep -q 'agentboard' "$codex_home/config.toml"; then
  echo "note: merge $src/codex/base.config.toml into $codex_home/config.toml" >&2
fi
echo "done: the codex base config is merged by hand; see seats/README.md"
