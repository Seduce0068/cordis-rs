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
check_file README.md "再利用 port（${ports}）" "ports"
check_file docs/PROGRESS.md "現状 ${ports} port" "ports"

# Do not let a stale count survive in a second sentence: fail if any line that
# mentions a port count disagrees with the computed value (R4 review).
for file in README.md docs/PROGRESS.md docs/ADR-001-cordis-runtime-reimplementation.md; do
  stale=$(grep -nE '(再利用 port|現状(は)?|全) [0-9]+ ?port|port [0-9]+ 個|port（[0-9]+）' "$file" \
    | grep -vE " ${ports} ?port|port ${ports} 個|port（${ports}）" || true)
  if [ -n "$stale" ]; then
    echo "docs drift: stale port count in $file:" >&2
    echo "$stale" >&2
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  echo "Actual counts: tests=$tests ports=$ports" >&2
  exit 1
fi
echo "doc counts verified: tests=$tests ports=$ports"
