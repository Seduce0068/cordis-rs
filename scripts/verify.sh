#!/usr/bin/env bash
# Full local verification loop matching the CI gates, in the same order.
# Update together with .github/workflows/ci.yml and rust-gates.yml@v1.
set -euo pipefail

echo "=== fmt ==="
cargo fmt --all --check

echo "=== check ==="
cargo check --all-targets --locked

echo "=== test ==="
cargo test --all-targets --locked

echo "=== clippy ==="
# Keep flags in sync with `clippy_args` in .github/workflows/ci.yml.
cargo clippy --all-targets --locked -- -D warnings -A clippy::type_complexity -D clippy::undocumented_unsafe_blocks

echo "=== cargo audit ==="
if command -v cargo-audit >/dev/null 2>&1; then
	cargo audit
else
	echo "cargo-audit not installed; skipping (CI installs it). Install: cargo install cargo-audit" >&2
fi

# Same order as `extra_gates` in .github/workflows/ci.yml.
echo "=== examples smoke ==="
bash "$(dirname "$0")/smoke-examples.sh"

echo "=== no unsafe ==="
bash "$(dirname "$0")/check-no-unsafe.sh"

echo "=== doc counts ==="
bash "$(dirname "$0")/check-doc-counts.sh"

# Local-only gate (not part of CI).
if command -v markdownlint-cli2 >/dev/null 2>&1; then
	echo "=== markdownlint (local only, not in CI) ==="
	markdownlint-cli2 README.md docs/index.md docs/ADR-001-cordis-runtime-reimplementation.md docs/PROGRESS.md
fi

echo "all verification gates passed"
