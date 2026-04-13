#!/usr/bin/env bash
# Sign a macOS binary with the Developer ID Application certificate +
# hardened runtime + entitlements. Required for keyring access on signed
# release builds and for notarization.
#
# Usage: .github/scripts/sign-macos.sh <binary-path>
#
# Environment:
#   CODESIGN_IDENTITY   — override the signing identity
#                         (default: "Developer ID Application: HANDO K.K. (Y53RSUA3SM)")
#   ENTITLEMENTS_FILE   — override entitlements path
#                         (default: <repo-root>/crates/claudepot-cli/macos/entitlements.plist)

set -euo pipefail

BINARY="${1:?usage: sign-macos.sh <binary-path>}"

if [[ ! -f "$BINARY" ]]; then
    echo "error: binary not found: $BINARY" >&2
    exit 1
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "error: signing is macOS-only; uname=$(uname -s)" >&2
    exit 1
fi

# Script lives at .github/scripts/sign-macos.sh — repo root is two levels up.
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
IDENTITY="${CODESIGN_IDENTITY:-Developer ID Application: HANDO K.K. (Y53RSUA3SM)}"
ENTITLEMENTS="${ENTITLEMENTS_FILE:-$REPO_ROOT/crates/claudepot-cli/macos/entitlements.plist}"

if [[ ! -f "$ENTITLEMENTS" ]]; then
    echo "error: entitlements file not found: $ENTITLEMENTS" >&2
    exit 1
fi

echo "signing: $BINARY"
echo "  identity:     $IDENTITY"
echo "  entitlements: $ENTITLEMENTS"

codesign --force --timestamp \
    --sign "$IDENTITY" \
    --options runtime \
    --entitlements "$ENTITLEMENTS" \
    "$BINARY"

echo "verifying:"
codesign --verify --verbose=2 "$BINARY"
codesign --display --entitlements - --xml "$BINARY" 2>/dev/null | head -20 || true

echo "signed."
