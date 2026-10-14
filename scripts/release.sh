#!/usr/bin/env bash
# Builds the XCFramework, zips it, computes its SPM checksum, creates a
# GitHub release for tag core-vX.Y.Z, and rewrites Package.swift's release
# binaryTarget to point at the new asset + checksum.
#
# Needs: gh (authenticated), swift (for `package compute-checksum`).
# Run from a `core-vX.Y.Z` tag: `git tag core-v0.1.0 && scripts/release.sh`.
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="${1:-$(git describe --tags --exact-match 2>/dev/null || true)}"
if [[ -z "$TAG" ]]; then
  echo "error: no tag given and HEAD is not exactly tagged (expected core-vX.Y.Z)" >&2
  exit 1
fi

bash scripts/build-xcframework.sh

ZIP="build/pocket_ffiFFI.xcframework.zip"
rm -f "$ZIP"
(cd build && zip -qry "$(basename "$ZIP")" "pocket_ffiFFI.xcframework")

CHECKSUM=$(swift package compute-checksum "$ZIP")
echo "checksum: $CHECKSUM"

gh release create "$TAG" "$ZIP" \
  --title "$TAG" \
  --notes "Core release $TAG. XCFramework checksum: $CHECKSUM"

ASSET_URL=$(gh release view "$TAG" --json assets -q '.assets[0].url')
echo "asset: $ASSET_URL"

echo "Update Package.swift's release binaryTarget (url=$ASSET_URL, checksum=$CHECKSUM) and commit."
