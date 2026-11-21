#!/usr/bin/env bash
# CI gate: docs must report the exact number of #[test] attributes and ports.
# Test count covers both integration tests (tests/*.rs) and unit tests in src/.
set -euo pipefail

cd "$(dirname "$0")/.."

integration=$(grep -rhoE '#\[(tokio::)?test\]' tests/*.rs | wc -l | tr -d ' ')
unit=$(grep -rhoE '#\[(tokio::)?test\]' src/ --include='*.rs' | wc -l | tr -d ' ')
tests=$((integration + unit))
ports=$(find src/ports -maxdepth 1 -name '*.rs' ! -name 'mod.rs' | wc -l | tr -d ' ')
services=$(find src/services -maxdepth 1 -name '*.rs' ! -name 'mod.rs' | wc -l | tr -d ' ')
# messageFeedback is provided via a port rather than a services/ module.
core_services=$((services + 1))

fail=0
check_file() {
  local file=$1 expected=$2 label=$3
  if ! grep -qF "$expected" "$file"; then
    echo "docs drift: $file does not report $label=$expected" >&2
    fail=1
  fi
}

check_file README.md "テスト ${tests} 件（統合 ${integration} ＋ unit ${unit}）" "tests"
check_file docs/PROGRESS.md "テスト ${tests} 件（統合 ${integration} ＋ unit ${unit}）" "tests"
check_file docs/ADR-001-cordis-runtime-reimplementation.md "全 ${tests} テスト通過（統合 ${integration} ＋ unit ${unit}）" "tests"
check_file README.md "再利用 port（${ports}）" "ports"
check_file docs/PROGRESS.md "現状 ${ports} port" "ports"
check_file README.md "コアサービス（${core_services}）" "core services"
check_file docs/PROGRESS.md "コアサービス ${core_services} 個" "core services"
check_file docs/ADR-001-cordis-runtime-reimplementation.md "コアサービス ${core_services} 個" "core services"

# Also fail on a stale test count in any other sentence (R1-CF01). Lines
# spelling out the breakdown (integration/unit) are allowed.
for file in README.md docs/PROGRESS.md docs/ADR-001-cordis-runtime-reimplementation.md; do
  stale=$(grep -nE '(テスト|全) [0-9]+ ?(件|テスト)' "$file" \
    | grep -vE " ${tests} ?(件|テスト)|統合 ${integration}|unit ${unit}" || true)
  if [ -n "$stale" ]; then
    echo "docs drift: stale test count in $file:" >&2
    echo "$stale" >&2
    fail=1
  fi
done

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
  echo "Actual counts: tests=$tests (integration=$integration unit=$unit) ports=$ports" >&2
  exit 1
fi
echo "doc counts verified: tests=$tests (integration=$integration unit=$unit) ports=$ports core_services=$core_services (modules=$services + messageFeedback)"
