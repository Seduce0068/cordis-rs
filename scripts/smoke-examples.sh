#!/usr/bin/env bash
# Run every cargo example as a headless smoke test.
set -euo pipefail

EXAMPLES=(
	agent_demo
	agent_checkpoint_demo
	load_cordis_yml
	mount_composition
	composition_with_tool_fs
	mount_real_composition
	permission_flow_demo
)

for example in "${EXAMPLES[@]}"; do
	echo "=== ${example} ==="
	timeout 60 cargo run --quiet --example "${example}"
	echo "ok: ${example}"
done

echo "all ${#EXAMPLES[@]} examples passed"
