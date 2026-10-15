#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"

echo "Building test-server helper binary..."
cargo build -p pocket-ffi --bin test-server

SERVER_BIN="$TARGET_DIR/debug/test-server"
LOG="$(mktemp)"

echo "Launching test server in background..."
"$SERVER_BIN" > "$LOG" 2>&1 &
SERVER_PID=$!
trap 'kill "$SERVER_PID" 2>/dev/null || true; rm -f "$LOG"' EXIT

PORT=""
KEY=""
for i in $(seq 1 100); do
  if grep -q "^READY " "$LOG"; then
    LINE=$(grep "^READY " "$LOG" | head -n1)
    PORT=$(echo "$LINE" | cut -d' ' -f2)
    KEY=$(echo "$LINE" | cut -d' ' -f3)
    break
  fi
  sleep 0.1
done

if [ -z "$PORT" ]; then
  echo "Failed to start test-server. Server log:"
  cat "$LOG"
  exit 1
fi

echo "Test server READY on port $PORT"
export OPENAI_BASE_URL="http://127.0.0.1:${PORT}/v1"
export OPENAI_API_KEY="${KEY}"

echo "=== Running Python SDK compatibility tests ==="
VENV_DIR="$ROOT/tests/sdk/python/.venv"
if [ ! -d "$VENV_DIR" ]; then
  python3 -m venv "$VENV_DIR"
fi
"$VENV_DIR/bin/pip" install -q -r "$ROOT/tests/sdk/python/requirements.txt"
"$VENV_DIR/bin/pytest" -v "$ROOT/tests/sdk/python/test_openai.py"

echo "=== Running Node SDK compatibility tests ==="
(cd "$ROOT/tests/sdk/node" && npm ci && node --test test.mjs)

echo "=== All SDK compatibility tests passed! ==="
