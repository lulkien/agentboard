#!/usr/bin/env bash
# The AGENTS.md proof table as one command: every gate, in order, with its own
# output, stopping at the first failure. Run it before calling a change done.
#
#   bash scripts/gate.sh                         # every gate but latency
#   bash scripts/gate.sh --full                  # also measure claim_latency
#   bash scripts/gate.sh --features dashboard    # prove the optional dashboard
#
# Every gate must also print what the table beside it promises. A gate that
# exits 0 with the wrong text is still a failure, because that text is what the
# counts in AGENTS.md are read against; checking it here is what stops the
# counts drifting by hand.
set -u

usage() {
  echo "usage: ${0##*/} [--full] [--features dashboard]"
}

full=0
dashboard_feature=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --full) full=1 ;;
    --features)
      shift
      if [ "${1:-}" != "dashboard" ]; then
        echo "gate: only the dashboard feature has a feature gate" >&2
        usage >&2
        exit 2
      fi
      dashboard_feature=1
      ;;
    --features=dashboard) dashboard_feature=1 ;;
    -h|--help) usage; exit 0 ;;
    *)
      echo "gate: unknown argument ${1}" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

# Runs from the repo root whatever directory the caller is in.
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd) || exit 1
cd "$root" || exit 1
feature_bin="$root/target/release/agentboard"

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

binary_gate() { # use the release binary built for this gate mode
  if [ "$dashboard_feature" -eq 1 ]; then
    gate env AGENTBOARD_TEST_BIN="$feature_bin" "$@"
  else
    gate "$@"
  fi
}

want() { # want <extended regex the command's output must contain>
  if ! grep -Eq -- "$1" "$log"; then
    printf 'GATE FAILED: %s (output lacks %s)\n' "$current" "$1" >&2
    exit 1
  fi
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

gate cargo fmt --check
silent

if [ "$dashboard_feature" -eq 1 ]; then
  gate cargo test --offline --lib --features dashboard
else
  gate cargo test --offline --lib
fi
want '^test result: ok\. [0-9]+ passed; 0 failed'

if [ "$dashboard_feature" -eq 1 ]; then
  gate cargo test --offline --bin agentboard --features dashboard
  want '^test result: ok\. 2 passed; 0 failed; 0 ignored'
fi

if [ "$dashboard_feature" -eq 1 ]; then
  gate cargo clippy --offline --all-targets --all-features -- -D warnings
else
  gate cargo clippy --offline --all-targets -- -D warnings
fi
refuse '^(warning|error)(\[|:)'

if [ "$dashboard_feature" -eq 1 ]; then
  gate cargo build --release --features dashboard
else
  gate cargo build --release
fi
want 'Finished'

binary_gate bash tests/cli.sh
want '^[0-9]+/[0-9]+ passed \(cli, lmdb\)$'

binary_gate python3 tests/mcp_client.py
want '^[0-9]+/[0-9]+ passed \(mcp, lmdb\)$'

if [ "$dashboard_feature" -eq 1 ]; then
  binary_gate python3 tests/dashboard.py
  want '^dashboard: all checks passed \(read-only HTTP, filters, paging, escaping, bounds\)$'
fi

binary_gate bash tests/race.sh
want '^ok: [0-9]+ processes claimed [0-9]+ tasks'

gate bash tests/skills-install.sh
want '^[0-9]+/[0-9]+ passed \(skills installer\)$'

gate bash tests/seats-install.sh
want '^[0-9]+/[0-9]+ passed \(seats installer\)$'

gate python3 scripts/sync-tool-count.py --check
want '^MCP router: [0-9]+ tools; published counts match$'

if [ "$dashboard_feature" -eq 1 ]; then
  gate python3 tests/doc-names.py --binary "$feature_bin"
else
  gate python3 tests/doc-names.py
fi
want '^doc-names: [0-9]+ pages, [0-9]+ CLI names, [0-9]+ MCP names passed$'

gate python3 scripts/doc-overlap.py
refuse '^FAIL'

if [ "$full" -eq 1 ]; then
  gate cargo test --offline --lib -- --ignored --nocapture claim_latency
  want '^claim latency, 8 processes over 1000 tasks: p50=[0-9.]+ms p95=[0-9.]+ms$'
else
  echo 'skip: claim_latency (--full)'
fi

printf 'GATES OK: %s gates passed\n' "$gates"
