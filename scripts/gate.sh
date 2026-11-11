#!/usr/bin/env bash
# The AGENTS.md proof table as one command: every gate, in order, with its own
# output, stopping at the first failure. Run it before calling a change done.
#
#   bash scripts/gate.sh                         # every gate but latency
#   bash scripts/gate.sh --full                  # also measure claim_latency
#   bash scripts/gate.sh --prebuilt              # suites/docs against release binary
#
# Pinned result lines are read from the AGENTS.md proof table: every backticked
# span in a pinned row's Expected cell is accepted as a prefix of the output
# line its locator regex finds. A count change fails until the table is edited.
set -u

usage() {
  echo "usage: ${0##*/} [--full | --prebuilt]"
}

full=0
prebuilt=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --full) full=1 ;;
    --prebuilt) prebuilt=1 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "gate: unknown argument ${1}" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

if [ "$full" -eq 1 ] && [ "$prebuilt" -eq 1 ]; then
  echo 'gate: --prebuilt cannot be combined with --full' >&2
  usage >&2
  exit 2
fi

# Runs from the repo root whatever directory the caller is in.
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd) || exit 1
cd "$root" || exit 1

if [ "$prebuilt" -eq 1 ]; then
  bin=${AGENTBOARD_TEST_BIN:-$root/target/release/agentboard}
  if [ ! -x "$bin" ]; then
    printf 'GATE FAILED: prebuilt binary is missing or not executable: %s\n' "$bin" >&2
    exit 1
  fi
  if find src dashboard Cargo.toml Cargo.lock -newer "$bin" -print -quit | grep -q .; then
    printf 'GATE FAILED: stale prebuilt binary: %s\n' "$bin" >&2
    exit 1
  fi
  export AGENTBOARD_TEST_BIN="$bin"
  export PYTHONDONTWRITEBYTECODE=1
fi

log=$(mktemp)
trap 'rm -f "$log"' EXIT

gates=0
current=

fail() { # fail <exit code>
  printf 'GATE FAILED: %s (exit %s)\n' "$current" "$1" >&2
  exit "$1"
}

gate() { # gate <command...>
  current="$*"
  printf '== %s\n' "$current"
  "$@" 2>&1 | tee "$log"
  local code=${PIPESTATUS[0]}
  [ "$code" -eq 0 ] || fail "$code"
  gates=$((gates + 1))
}

want() { # want <extended regex the command's output must contain>
  if ! grep -Eq -- "$1" "$log"; then
    printf 'GATE FAILED: %s (output lacks %s)\n' "$current" "$1" >&2
    exit 1
  fi
}

expected() { # print the accepted result lines for a proof-table command key
  awk -F'|' -v key="$1" '
    /^\| / {
      command_count = split($2, command_parts, "`")
      if (command_count < 3 || command_parts[2] != key) next
      expected_count = split($3, expected_parts, "`")
      for (i = 2; i <= expected_count; i += 2) print expected_parts[i]
    }
  ' AGENTS.md
}

pin() { # pin <extended regex locating the result line in the output>
  local actual key accepted found=0
  actual=$(grep -E -- "$1" "$log" | tail -n 1)
  if [ -z "$actual" ]; then
    printf 'GATE FAILED: %s (output lacks %s)\n' "$current" "$1" >&2
    exit 1
  fi

  key=${current#env }
  accepted=$(expected "$key")
  if [ -z "$accepted" ]; then
    printf 'GATE FAILED: %s (AGENTS.md has no expected line for %s)\n' "$current" "$key" >&2
    exit 1
  fi

  while IFS= read -r want_line; do
    if [[ $actual == "$want_line"* ]]; then
      found=1
      break
    fi
  done <<< "$accepted"
  [ "$found" -eq 1 ] && return 0

  printf 'GATE FAILED: %s (output does not match AGENTS.md)\n' "$current" >&2
  while IFS= read -r want_line; do
    printf '  expected: %s\n' "$want_line" >&2
  done <<< "$accepted"
  printf '  actual:   %s\n' "$actual" >&2
  exit 1
}

refuse() { # refuse <extended regex that must match nothing>
  if grep -Eq -- "$1" "$log"; then
    printf 'GATE FAILED: %s (output matches %s)\n' "$current" "$1" >&2
    exit 1
  fi
}

silent() { # the command must print nothing at all
  if [ -s "$log" ]; then
    printf 'GATE FAILED: %s (expected no output)\n' "$current" >&2
    exit 1
  fi
}

if [ "$prebuilt" -eq 0 ]; then
  gate cargo fmt --check
  silent

  gate cargo test --offline --lib
  pin '^test result: '

  gate cargo clippy --offline --all-targets -- -D warnings
  refuse '^(warning|error)(\[|:)'

  gate cargo build --release
  want 'Finished'
fi

gate bash tests/cli.sh
pin '^[0-9]+/[0-9]+ passed \(cli, lmdb\)$'

gate python3 tests/mcp_client.py
pin '^[0-9]+/[0-9]+ passed \(mcp, lmdb\)$'

gate env AGENTBOARD_MCP_TRANSPORT=http python3 tests/mcp_client.py
pin '^[0-9]+/[0-9]+ passed \(mcp-http, lmdb\)$'

gate python3 tests/daemon.py
pin '^daemon: [0-9]+/[0-9]+ passed \(http, lmdb\)$'

gate bash tests/race.sh
pin '^ok: [0-9]+ processes claimed'

gate bash tests/migrate.sh
pin '^(migrate: [0-9]+/[0-9]+ passed \(scratch LMDB, stepwise\)|SKIP migrate: Python lmdb unavailable offline \(use uv run --with lmdb\))$'

gate bash tests/skills-install.sh
pin '^[0-9]+/[0-9]+ passed \(skills installer\)$'

gate bash tests/seats-install.sh
pin '^[0-9]+/[0-9]+ passed \(seats installer\)$'

gate bash tests/start-agentboard-manager.sh
pin '^[0-9]+/[0-9]+ passed \(manager launcher\)$'

gate python3 scripts/sync-tool-count.py --check
pin '^MCP router: [0-9]+ tools; published counts match$'

gate python3 tests/doc-names.py
pin '^doc-names: [0-9]+ pages, [0-9]+ CLI names, [0-9]+ MCP names passed$'

gate python3 scripts/doc-overlap.py
refuse '^FAIL'

if [ "$full" -eq 1 ]; then
  gate cargo test --offline --lib -- --ignored --nocapture claim_latency
  want '^claim latency, 8 processes over 1000 tasks: p50=[0-9.]+ms p95=[0-9.]+ms$'
else
  echo 'skip: claim_latency (--full)'
fi

printf 'GATES OK: %s gates passed\n' "$gates"
