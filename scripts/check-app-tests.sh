#!/usr/bin/env bash
# UI-144: prove the app test targets still COMPILE (build-for-testing, nothing is run).
# Run after changing CoreClient, the pocket-ffi surface, or anything a test fake
# (Mac Tests/TestCore.swift) conforms to: a missing fake method breaks the Mac
# test target silently, because the app build does not compile tests.
# Xcode runs only on the remote build Mac (`laptop`); see ~/.claude/CLAUDE.md.
# Checks: PocketAI.Mac scheme PocketAI (PocketAITests + PocketAIUITests, macOS) and
# PocketAI.Iphone scheme PocketAI (PocketAIUITests, iOS Simulator; the iPhone has
# no unit-test target). Prints PASS/FAIL and exits non-zero on any failure.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HOST=laptop
DEST='~/pocketai-build'
cd "$ROOT"

rsync -az --delete --exclude .git --exclude .build --exclude .swiftpm --exclude target \
  --exclude DerivedData --exclude node_modules --exclude 'build-*' \
  --exclude '/PocketAI.Mac/build' --exclude '/PocketAI.Iphone/build' \
  --exclude '/PocketAI.Core/build/*' --exclude '*.xcodeproj' \
  PocketAI.Core PocketAI.Mac PocketAI.Iphone "$HOST:pocketai-build/"

fail=0
check() { # name, project dir, xcodebuild destination args
  local name=$1 dir=$2 dd=$3; shift 3
  if ssh "$HOST" zsh -l -s <<REMOTE | grep -E 'error:|BUILD (SUCCEEDED|FAILED)|TEST BUILD'
set -o pipefail
cd $DEST/$dir && xcodegen generate -q && \
xcodebuild build-for-testing -scheme PocketAI -derivedDataPath $DEST/$dd -jobs 4 \
  CODE_SIGNING_ALLOWED=NO $* 2>&1
st=\$?
rm -rf $DEST/$dd
exit \$st
REMOTE
  then
    # grep succeeded only means output matched; the real verdict is ssh's status (pipefail).
    if [ "${PIPESTATUS[0]}" -eq 0 ]; then echo "PASS: $name"; else echo "FAIL: $name"; fail=1; fi
  else
    echo "FAIL: $name"; fail=1
  fi
}

check "Mac tests compile (PocketAITests, PocketAIUITests)" PocketAI.Mac dd-tests-mac \
  -destination 'platform=macOS' -configuration Debug
check "iPhone UI tests compile (PocketAIUITests)" PocketAI.Iphone dd-tests-sim \
  -destination 'generic/platform=iOS\ Simulator' ARCHS=arm64 ONLY_ACTIVE_ARCH=YES

if [ "$fail" -eq 0 ]; then echo "PASS: app test targets compile"; else echo "FAIL: app test targets do not compile"; fi
exit "$fail"
