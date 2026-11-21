#!/usr/bin/env bash
# CI gate: reject `unsafe` in Rust source (AGENTS.md §15).
# Comments and string literals that merely mention the word are allowed.
set -euo pipefail

# Scan the whole repository (excluding build artifacts) so benches/, build.rs
# and any future Rust source cannot bypass the gate (swarm review R4-CI03).
# The pattern also covers `unsafe extern` forms.
rust_files=$(find . -path './target' -prune -o -name '*.rs' -print)
hits=$(printf '%s\n' "$rust_files" | xargs -r grep -HnE '(^|[[:space:]])(pub[[:space:]]+)?(async[[:space:]]+)?unsafe[[:space:]]+(extern[[:space:]]+("[^"]*"[[:space:]])?(fn|block)|fn|impl|trait|block)|(^|[[:space:]])unsafe[[:space:]]*\{' || true)
if [ -n "$hits" ]; then
  echo "unsafe Rust usage is forbidden (AGENTS.md §15):" >&2
  echo "$hits" >&2
  exit 1
fi
echo "no unsafe Rust usage found"

# CI gate: a raw Fiber::mount result must be retained in src/ports.
# Fiber::mount_retained keeps the fiber in an effect itself, so discarding its
# return value (the common `let _ = ...mount_retained(...)` form) is safe and
# must not be flagged. `Fiber::mount\(` exact match excludes mount_retained
# naturally. loader.rs is out of scope: it returns Vec<Fiber> by design.
ghost=$(grep -RIn --include='*.rs' -E 'Fiber::mount\(' src/ports 2>/dev/null || true)
if [ -n "$ghost" ]; then
  echo "raw Fiber::mount result must be retained in ports (use mount_retained):" >&2
  echo "$ghost" >&2
  exit 1
fi

# Self-test: ensure the unsafe pattern detects all forbidden forms, including
# a line-start `unsafe {` (swarm review R5-CI04).
for forbidden in 'unsafe { let x = 1; }' '    unsafe { let x = 1; }' 'pub unsafe extern "C" fn f() {}'; do
  if ! printf '%s\n' "$forbidden" | grep -qE '(^|[[:space:]])(pub[[:space:]]+)?(async[[:space:]]+)?unsafe[[:space:]]+(extern[[:space:]]+("[^"]*"[[:space:]])?(fn|block)|fn|impl|trait|block)|(^|[[:space:]])unsafe[[:space:]]*\{'; then
    echo "unsafe gate self-test failed for: $forbidden" >&2
    exit 1
  fi
done
echo "unsafe gate self-test passed"

# Self-test: ensure the pattern actually detects a discarded mount so this
# gate cannot silently become a dead check (swarm review R4-DP01).
self_test_dir=$(mktemp -d)
trap 'rm -rf "$self_test_dir"' EXIT
printf '%s\n' 'let _ = crate::Fiber::mount(ctx, plugin).await?;' > "$self_test_dir/ghost.rs"
if ! grep -q -E 'Fiber::mount\(' "$self_test_dir/ghost.rs"; then
  echo "ghost-fiber gate self-test failed" >&2
  exit 1
fi
echo "no raw Fiber::mount results discarded in ports"
