#!/usr/bin/env bash
# Run every cargo example as a headless smoke test.
# Convention: every .rs directly under examples/ is a runnable smoke target.
set -euo pipefail

mapfile -t examples < <(find examples -maxdepth 1 -name '*.rs' -exec basename {} .rs \; | sort)

# A zero-target run is not a passing smoke gate: it means examples/ was
# deleted or find failed, and the gate would silently pass (R4-DP09).
if [ "${#examples[@]}" -eq 0 ]; then
	echo "FAIL: no cargo examples found under examples/; smoke gate cannot pass" >&2
	exit 1
fi

for example in "${examples[@]}"; do
	echo "=== ${example} ==="
	timeout 60 cargo run --quiet --example "${example}"
	echo "ok: ${example}"
done

echo "all ${#examples[@]} examples passed"
