#!/usr/bin/env bash
# CI gate: reject secrets with Betterleaks (AGENTS.md §4); gitleaks remains as
# an auxiliary scanner below.
#
# This is the final gate: a pre-commit hook (see .pre-commit-config.yaml) can
# be bypassed with `git commit --no-verify`, so CI must always run this check.
# --no-banner / --redact keep a detected secret out of the CI log itself.
set -euo pipefail

GITLEAKS_VERSION="8.28.0"
# SHA-256 pinned from the official v8.28.0 release checksums.txt. Pin the hash
# in this script (not by re-downloading checksums.txt) so a compromised origin
# cannot forge both the archive and its checksum (swarm review R4-CI01 R1).
GITLEAKS_SHA256="a65b5253807a68ac0cafa4414031fd740aeb55f54fb7e55f386acb52e6a840eb"
TMP_DIR=""

cleanup() {
  if [ -n "$TMP_DIR" ] && [ -d "$TMP_DIR" ]; then
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

install_gitleaks_ci() {
  TMP_DIR=$(mktemp -d)
  local archive="$TMP_DIR/gitleaks.tgz"
  local url="https://github.com/gitleaks/gitleaks/releases/download/v${GITLEAKS_VERSION}/gitleaks_${GITLEAKS_VERSION}_linux_x64.tar.gz"
  curl -fsSL "$url" -o "$archive"
  echo "$GITLEAKS_SHA256  $archive" | sha256sum --check -
  tar -xzf "$archive" -C "$TMP_DIR" gitleaks
  export PATH="$TMP_DIR:$PATH"
}

if ! command -v gitleaks >/dev/null 2>&1; then
  if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
    install_gitleaks_ci
  else
    echo "gitleaks is required for the secret scan gate; install via" >&2
    echo "  pre-commit install                     (local, see .pre-commit-config.yaml)" >&2
    echo "  GitHub Releases binary                 (https://github.com/gitleaks/gitleaks/releases)" >&2
    echo "  taiki-e/install-action@tool=gitleaks   (CI)" >&2
    exit 1
  fi
fi

if ! command -v gitleaks >/dev/null 2>&1; then
  echo "gitleaks installation failed" >&2
  exit 1
fi

# Scan working tree and full history. A narrow .gitleaks.toml allowlist may be
# added later for a documented test fixture; keep it path-specific and dated.
gitleaks detect --no-banner --redact --no-git
gitleaks detect --no-banner --redact

# Betterleaks v1.10.1 is the primary gate (AGENTS.md §4). CI installs it via
# .github/workflows/ci.yml -> secret-gates.yml@v1 and scans the full history;
# run the same check here when the binary is available. `set -e` above makes a
# non-zero exit fail closed.
if command -v betterleaks >/dev/null 2>&1; then
  betterleaks git --no-banner .
fi

echo "no secrets found"
