#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if python3 -c 'import lmdb' >/dev/null 2>&1; then
  python3 tests/migrate.py
elif command -v uv >/dev/null 2>&1 && uv run --with lmdb --offline python3 -c 'import lmdb' >/dev/null 2>&1; then
  uv run --with lmdb --offline python3 tests/migrate.py
else
  echo 'SKIP migrate: Python lmdb unavailable offline (use uv run --with lmdb)'
fi
