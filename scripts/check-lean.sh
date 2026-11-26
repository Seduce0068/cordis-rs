#!/usr/bin/env bash
# CI gate: verify the Lean formalization under formal/ (swarm R2-CHECKSCRIPT).
#
# `lake build --wfail` turns Lean warnings into errors; `lake env leanchecker`
# re-checks the compiled environment of the CaddyFormal module with the native
# kernel checker. Anchor at the repo root first so local runs and GitHub
# Actions runs use exactly the same paths.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -d formal ]; then
  echo "formal/ directory is required for the Lean gate" >&2
  exit 1
fi

# elan installs lake/lean in ~/.elan/bin; CI may already have it on PATH, but
# prepending keeps local and CI behavior identical.
export PATH="$HOME/.elan/bin:$PATH"

if ! command -v lake >/dev/null 2>&1; then
  echo "lake (Lean package manager from elan) is required for the Lean gate" >&2
  echo "  install elan: https://github.com/leanprover/elan" >&2
  exit 1
fi

cd formal

# Build the explicit target so an empty default-target set cannot silently skip
# compilation. `--wfail` turns Lean warnings into errors.
lake build --wfail CaddyFormal

# A zero-target build exits 0 while producing no oleans; verify the artifacts
# before treating the gate as passed.
olean_dir=".lake/build/lib/lean"
if [ ! -f "$olean_dir/CaddyFormal.olean" ] || [ ! -f "$olean_dir/Caddy.olean" ]; then
  echo "Lean build produced no oleans for CaddyFormal/Caddy; gate cannot pass" >&2
  exit 1
fi

# The theorems live in Caddy.lean, so check that module directly instead of
# relying on a re-export wrapper. Capture output to detect an empty check.
checker_out="$(lake env leanchecker Caddy 2>&1)" || {
  printf '%s\n' "$checker_out" >&2
  exit 1
}
printf '%s\n' "$checker_out"
case "$checker_out" in
  *"Could not find any oleans"*)
    echo "leanchecker found no oleans; gate cannot pass" >&2
    exit 1
    ;;
esac

echo "Lean proofs pass lake build --wfail CaddyFormal and leanchecker Caddy"
