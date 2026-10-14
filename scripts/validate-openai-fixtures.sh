#!/usr/bin/env bash
# Structural sanity check for contracts/openai/*. Full validation against the
# published OpenAI OpenAPI spec needs network access to fetch a pinned spec
# commit; not available in this environment (see DECISIONS.md). This script
# checks required top-level keys per fixture instead.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 -I - <<'PY'
import json, sys
from pathlib import Path

base = Path("contracts/openai")
errors = []

def require(obj, keys, ctx):
    for k in keys:
        if k not in obj:
            errors.append(f"{ctx}: missing key '{k}'")

def check_json(path, keys):
    data = json.loads(path.read_text())
    require(data, keys, path.name)

check_json(base / "models.list.json", ["object", "data"])
check_json(base / "chat.nonstream.req.json", ["model", "messages"])
check_json(base / "chat.nonstream.res.json", ["id", "object", "choices", "usage"])
check_json(base / "chat.stream.req.json", ["model", "messages", "stream"])
check_json(base / "chat.tools.req.json", ["model", "messages", "tools"])
check_json(base / "chat.tools.res.json", ["choices"])
check_json(base / "chat.vision.req.json", ["model", "messages"])
check_json(base / "embeddings.req.json", ["model", "input"])
check_json(base / "embeddings.res.json", ["object", "data", "usage"])
check_json(base / "error.401.json", ["error"])
check_json(base / "error.400.json", ["error"])
check_json(base / "error.404.model.json", ["error"])
check_json(base / "audio.transcriptions.res.json", ["text"])
check_json(base / "images.generations.req.json", ["model", "prompt"])
check_json(base / "images.generations.res.json", ["created", "data"])

for sse in ["chat.stream.res.sse", "chat.reasoning.stream.res.sse"]:
    text = (base / sse).read_text()
    if not text.strip().endswith("data: [DONE]"):
        errors.append(f"{sse}: must end with 'data: [DONE]'")
    if "chat.completion.chunk" not in text:
        errors.append(f"{sse}: missing chat.completion.chunk events")

if errors:
    for e in errors:
        print(f"FAIL: {e}", file=sys.stderr)
    sys.exit(1)
print(f"OK: {14} JSON fixtures + 2 SSE fixtures validated (structural check only)")
PY
