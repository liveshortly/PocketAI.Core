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

for t in aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin; do
  cargo +stable build -p pocket-ffi --release --target "$t"
done

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
