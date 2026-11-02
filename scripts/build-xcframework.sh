#!/usr/bin/env bash
set -euo pipefail
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
# Honor CARGO_TARGET_DIR (CI points it at a persistent cache dir); default
# to the in-repo target/ for local builds.
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
OUT="$ROOT/build"
HEADERS="$OUT/headers"
XC="$OUT/pocket_ffiFFI.xcframework"

rm -rf "$HEADERS" "$XC"
mkdir -p "$HEADERS" "$OUT/bindings"

# Deployment targets, pinned to match swift/Package.swift
# (platforms: [.iOS(.v18), .macOS(.v14)]) and both apps' project.yml.
#
# Without these, clang stamps the host SDK's version into the static libs:
# the macOS slice came out minos 15.0 against an app targeting 14.0, so every
# link emitted an object-file-version mismatch warning and macOS 14 was never
# actually the thing being built. They are pinned rather than inherited so a
# new Xcode cannot move them again.
export MACOSX_DEPLOYMENT_TARGET=14.0
export IPHONEOS_DEPLOYMENT_TARGET=18.0
# llama-cpp-sys-2 builds llama.cpp through cmake, which reads its own variable
# and ignores MACOSX_DEPLOYMENT_TARGET — without this the several hundred
# llama.cpp C++ objects inside libpocket_ffi.a keep the host SDK's version.
export CMAKE_OSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET

# Cargo's fingerprint does not include these env vars, so changing one
# rebuilds pocket-ffi and leaves every cached dependency object stamped with
# the old version — which is how the macOS slice ended up reporting 14.0 while
# 286 dependency objects inside it still said 15.0. Clean the target when the
# pinned versions change, and only then, so an unchanged local build stays
# incremental.
STAMP="$TARGET_DIR/.deployment-targets"
WANT="macos=$MACOSX_DEPLOYMENT_TARGET ios=$IPHONEOS_DEPLOYMENT_TARGET"
if [ "$(cat "$STAMP" 2>/dev/null || true)" != "$WANT" ]; then
  echo "Deployment targets changed ($WANT) — cleaning so dependencies restamp"
  for t in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin; do
    cargo +stable clean --release --target "$t"
  done
  # The host dir too, and it matters: on an Apple Silicon Mac the host triple
  # IS aarch64-apple-darwin, so cargo serves the darwin build from artifacts
  # cached here. Cleaning only the per-target dirs left ~290 objects inside
  # the macOS slice still stamped with the old version.
  cargo +stable clean --release
fi

for t in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin; do
  cargo +stable build -p pocket-ffi --release --target "$t"
done

mkdir -p "$(dirname "$STAMP")"
printf '%s' "$WANT" > "$STAMP"

cargo run -p pocket-ffi --bin uniffi-bindgen -- generate \
  --library "$TARGET_DIR/aarch64-apple-darwin/release/libpocket_ffi.dylib" \
  --language swift --out-dir "$OUT/bindings"

cp "$OUT/bindings/pocket_ffiFFI.h" "$HEADERS/"
cp "$OUT/bindings/pocket_ffiFFI.modulemap" "$HEADERS/module.modulemap"

xcodebuild -create-xcframework \
  -library "$TARGET_DIR/aarch64-apple-ios/release/libpocket_ffi.a" -headers "$HEADERS" \
  -library "$TARGET_DIR/aarch64-apple-ios-sim/release/libpocket_ffi.a" -headers "$HEADERS" \
  -library "$TARGET_DIR/aarch64-apple-darwin/release/libpocket_ffi.a" -headers "$HEADERS" \
  -output "$XC"

mkdir -p swift/Sources/PocketKit
cp "$OUT/bindings/pocket_ffi.swift" swift/Sources/PocketKit/pocket_ffi.swift

echo "Built $XC"
