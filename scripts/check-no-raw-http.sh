#!/usr/bin/env bash
# NET-15: only pocket-trust (PocketHttp) and pocket-models::hub may call
# reqwest:: directly. Every other crate must go through PocketHttp so
# offline-mode refusal and egress recording can't be bypassed.
set -euo pipefail
cd "$(dirname "$0")/.."

violations=$(grep -rn "reqwest::" crates --include="*.rs" \
  | grep -v "^crates/pocket-trust/" \
  | grep -v "^crates/pocket-models/src/hub.rs" \
  || true)

if [[ -n "$violations" ]]; then
  echo "error: raw reqwest:: use outside pocket-trust/pocket-models::hub:" >&2
  echo "$violations" >&2
  exit 1
fi
echo "OK: no raw reqwest:: use outside the allowed crates"
