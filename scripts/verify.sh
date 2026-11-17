#!/usr/bin/env bash
# Full local verification loop matching the CI gates, in the same order.
set -euo pipefail

echo "=== fmt ==="
cargo fmt --all --check

echo "=== check ==="
cargo check --all-targets

echo "=== clippy ==="
cargo clippy --all-targets -- -D warnings

echo "=== test ==="
cargo test --all-targets

echo "=== examples smoke ==="
bash "$(dirname "$0")/smoke-examples.sh"

if command -v markdownlint-cli2 >/dev/null 2>&1; then
	echo "=== markdownlint ==="
	markdownlint-cli2 README.md docs/index.md docs/ADR-001-cordis-runtime-reimplementation.md docs/PROGRESS.md
fi

echo "all verification gates passed"
