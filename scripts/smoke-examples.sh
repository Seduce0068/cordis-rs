#!/usr/bin/env bash
# Run every cargo example as a headless smoke test.
# Convention: every .rs directly under examples/ is a runnable smoke target.
set -euo pipefail

mapfile -t examples < <(find examples -maxdepth 1 -name '*.rs' -exec basename {} .rs \; | sort)

for example in "${examples[@]}"; do
	echo "=== ${example} ==="
	timeout 60 cargo run --quiet --example "${example}"
	echo "ok: ${example}"
done

echo "all ${#examples[@]} examples passed"
