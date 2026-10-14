#!/usr/bin/env bash
# CORE-13: signs crates/pocket-models/data/catalog.json (or any catalog JSON
# passed as $2) with the Ed25519 private key at $1 (32 raw bytes, hex-encoded
# in the file). Writes <file>.sig next to it. The private key never ships in
# this repo; keep it wherever getpocketai.app/catalog is published from.
set -euo pipefail
cd "$(dirname "$0")/.."

KEY_FILE="${1:?usage: sign-catalog.sh <private-key-hex-file> [catalog.json]}"
CATALOG_FILE="${2:-crates/pocket-models/data/catalog.json}"

cargo run -p pocket-models --bin sign-catalog -- "$KEY_FILE" "$CATALOG_FILE"
