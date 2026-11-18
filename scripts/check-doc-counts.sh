#!/usr/bin/env bash
# CI gate: docs must report the exact number of #[test] attributes and ports.
set -euo pipefail

cd "$(dirname "$0")/.."

tests=$(grep -rhoE '#\[(tokio::)?test\]' tests/*.rs | wc -l | tr -d ' ')
ports=$(find src/ports -maxdepth 1 -name '*.rs' ! -name 'mod.rs' | wc -l | tr -d ' ')

fail=0
check_file() {
  local file=$1 expected=$2 label=$3
  if ! grep -qF "$expected" "$file"; then
    echo "docs drift: $file does not report $label=$expected" >&2
    fail=1
  fi
}

check_file README.md "全 ${tests} 件" "tests"
check_file docs/PROGRESS.md "テスト ${tests} 件" "tests"
check_file docs/ADR-001-cordis-runtime-reimplementation.md "全 ${tests} テスト通過" "tests"

if [ "$fail" -ne 0 ]; then
  echo "Actual counts: tests=$tests ports=$ports" >&2
  exit 1
fi
echo "doc counts verified: tests=$tests ports=$ports"
