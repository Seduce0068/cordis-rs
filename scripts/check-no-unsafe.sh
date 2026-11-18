#!/usr/bin/env bash
# CI gate: reject `unsafe` in Rust source (AGENTS.md §15).
# Comments and string literals that merely mention the word are allowed.
set -euo pipefail

hits=$(grep -RIn --include='*.rs' -E '^[[:space:]]*(pub[[:space:]]+)?(async[[:space:]]+)?unsafe[[:space:]]+(fn|impl|trait|block)|[[:space:]]unsafe[[:space:]]*\{' src tests examples 2>/dev/null || true)
if [ -n "$hits" ]; then
  echo "unsafe Rust usage is forbidden (AGENTS.md §15):" >&2
  echo "$hits" >&2
  exit 1
fi
echo "no unsafe Rust usage found"

# CI gate: port mount helpers must retain their fiber (no ghost fibers).
ghost=$(grep -RIn --include='*.rs' 'let _fiber' src/ports 2>/dev/null || true)
if [ -n "$ghost" ]; then
  echo "port mount helpers must use Fiber::mount_retained (AGENTS.md §15 style):" >&2
  echo "$ghost" >&2
  exit 1
fi
echo "no ghost fiber mount helpers found"
