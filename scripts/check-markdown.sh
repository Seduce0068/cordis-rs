#!/usr/bin/env bash
# CI gate: lint every Markdown deliverable (swarm review R4-CI02).
#
# Local markdownlint-cli2 is preferred; on GitHub Actions (where it is not
# pre-installed) fall back to a version-pinned npx so local and CI use the
# same rule set. The glob is quoted so markdownlint-cli2 expands it, not bash.
set -euo pipefail

MARKDOWNLINT_VERSION="0.24.1"

if command -v markdownlint-cli2 >/dev/null 2>&1; then
  markdownlint-cli2 '**/*.md'
elif [ "${GITHUB_ACTIONS:-}" = "true" ] && command -v npx >/dev/null 2>&1; then
  npx --yes "markdownlint-cli2@${MARKDOWNLINT_VERSION}" '**/*.md'
else
  echo "markdownlint-cli2 (or npx on CI) is required for the Markdown gate" >&2
  exit 1
fi

echo "all Markdown files pass lint"
