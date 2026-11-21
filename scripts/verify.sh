#!/usr/bin/env bash
# Full local verification loop matching the CI gates, in the same order.
# Update together with .github/workflows/ci.yml and rust-gates.yml@v1.
set -euo pipefail

# Work from the repository root so the script is callable from any cwd.
cd "$(dirname "$0")/.."

echo "=== fmt ==="
cargo fmt --all --check

echo "=== check ==="
cargo check --all-targets --locked

echo "=== test ==="
cargo test --all-targets --locked

echo "=== clippy ==="
# Keep flags in sync with `clippy_args` in .github/workflows/ci.yml.
cargo clippy --all-targets --locked -- -D warnings -A clippy::type_complexity -D clippy::undocumented_unsafe_blocks

echo "=== secret scan ==="
bash scripts/check-secrets.sh

echo "=== cargo audit ==="
if command -v cargo-audit >/dev/null 2>&1; then
	cargo audit
elif [ "${AUDIT_SKIP:-}" = "1" ]; then
	echo "cargo-audit not installed; skipped by explicit AUDIT_SKIP=1" >&2
else
	echo "FAIL: cargo-audit is required (CI runs it). Install with 'cargo install cargo-audit'," >&2
	echo "      or set AUDIT_SKIP=1 to explicitly opt out of this local gate." >&2
	exit 1
fi

# Same order as `extra_gates` in .github/workflows/ci.yml.
echo "=== examples smoke ==="
bash scripts/smoke-examples.sh

echo "=== no unsafe ==="
bash scripts/check-no-unsafe.sh

echo "=== doc counts ==="
bash scripts/check-doc-counts.sh

echo "=== markdownlint (all Markdown deliverables) ==="
bash scripts/check-markdown.sh

echo "all verification gates passed"
