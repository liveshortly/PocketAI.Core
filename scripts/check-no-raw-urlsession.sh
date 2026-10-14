#!/usr/bin/env bash
# APL-04 (L8 CI lint): Net (and PeerClient, APL-09) must be the only places
# in the Swift app code that talk directly to URLSession - everywhere else
# goes through Net so offline mode and the egress ledger stay honest.
set -euo pipefail
cd "$(dirname "$0")/.."

violations=$(grep -rnE "URLSession\.shared|URLSession\(configuration:" \
  --include="*.swift" \
  swift/Sources ../PocketAI.Mac/Sources ../PocketAI.Iphone/Sources 2>/dev/null \
  | grep -vE "/(Net|Peer)/" || true)

if [[ -n "$violations" ]]; then
  echo "FAIL: raw URLSession use outside Net/ and Peer/:" >&2
  echo "$violations" >&2
  exit 1
fi
echo "OK: no raw URLSession use outside Net/ and Peer/"
