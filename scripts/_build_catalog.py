#!/usr/bin/env python3
"""One-off driver for catalog-pin.py's pin() to assemble data/catalog.json.
Scratch script, not part of the shipped CLI surface and not referenced by CI."""
import importlib.util
import json
import os
import sys

here = os.path.dirname(__file__)
spec = importlib.util.spec_from_file_location("catalog_pin", os.path.join(here, "catalog-pin.py"))
catalog_pin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(catalog_pin)
pin = catalog_pin.pin

SPECS = [
    # (repo, format, modalities, tags, gguf_file, quant, path_prefix, include)
    ("mlx-community/Qwen3-1.7B-4bit", "mlx", ["text"], ["recommended", "tier:6gb_iphone"], None, "4bit", None, None),
    ("mlx-community/gemma-3-1b-it-qat-4bit", "mlx", ["text"], ["tier:6gb_iphone"], None, "4bit", None, None),
    ("mlx-community/Llama-3.2-1B-Instruct-4bit", "mlx", ["text"], ["tier:6gb_iphone"], None, "4bit", None, None),
    ("mlx-community/LFM2-1.2B-4bit", "mlx", ["text"], ["tier:6gb_iphone"], None, "4bit", None, None),

    ("mlx-community/Qwen3-4B-Instruct-2507-4bit", "mlx", ["text"], ["recommended", "tier:8gb_iphone"], None, "4bit", None, None),
    ("mlx-community/Llama-3.2-3B-Instruct-4bit", "mlx", ["text"], ["tier:8gb_iphone"], None, "4bit", None, None),
    ("mlx-community/Phi-4-mini-instruct-4bit", "mlx", ["text"], ["tier:8gb_iphone"], None, "4bit", None, None),
    ("mlx-community/SmolLM3-3B-4bit", "mlx", ["text"], ["tier:8gb_iphone"], None, "4bit", None, None),
    ("mlx-community/gemma-3n-E4B-it-4bit", "mlx", ["text"], ["tier:8gb_iphone"], None, "4bit", None, None),

    ("mlx-community/Qwen3-8B-4bit", "mlx", ["text"], ["recommended", "tier:mac_16gb"], None, "4bit", None, None),
    ("mlx-community/Qwen3-14B-4bit", "mlx", ["text"], ["tier:mac_16gb"], None, "4bit", None, None),
    ("mlx-community/gpt-oss-20b-MXFP4-Q8", "mlx", ["text"], ["tier:mac_16gb"], None, "MXFP4", None, None),

    ("mlx-community/Qwen3-30B-A3B-Instruct-2507-4bit", "mlx", ["text"], ["recommended", "tier:mac_32gb_plus"], None, "4bit", None, None),
    ("mlx-community/Mistral-Small-3.2-24B-Instruct-2506-4bit", "mlx", ["text"], ["tier:mac_32gb_plus"], None, "4bit", None, None),

    ("mlx-community/SmolVLM2-500M-Video-Instruct-mlx", "mlx", ["vision"], [], None, "unknown", None, None),
    ("mlx-community/Qwen3-VL-4B-Instruct-4bit", "mlx", ["text", "vision"], [], None, "4bit", None, None),
    ("mlx-community/gemma-3-4b-it-qat-4bit", "mlx", ["text", "vision"], [], None, "4bit", None, None),
    ("mlx-community/Qwen2.5-VL-7B-Instruct-4bit", "mlx", ["text", "vision"], [], None, "4bit", None, None),
    ("mlx-community/gemma-3-27b-it-qat-4bit", "mlx", ["text", "vision"], [], None, "4bit", None, None),

    ("unsloth/Qwen3-4B-Instruct-2507-GGUF", "gguf", ["text"], [], "Qwen3-4B-Instruct-2507-Q4_K_M.gguf", "Q4_K_M", None, None),
    ("unsloth/Llama-3.2-3B-Instruct-GGUF", "gguf", ["text"], [], "Llama-3.2-3B-Instruct-Q4_K_M.gguf", "Q4_K_M", None, None),

    ("mlx-community/bge-small-en-v1.5-4bit", "mlx", ["embedding"], [], None, "4bit", None, None),

    # Mac: large-v3-v20240930 turbo variant; iPhone: small. Same repo, different subfolders.
    ("argmaxinc/whisperkit-coreml", "whisperkit", ["speech_to_text"], ["mac"], None, "unknown",
     "openai_whisper-large-v3-v20240930_turbo/", None),
    ("argmaxinc/whisperkit-coreml", "whisperkit", ["speech_to_text"], ["iphone"], None, "unknown",
     "openai_whisper-small/", None),

    # English ANE-optimized variant only (not ja/zh/v3).
    ("FluidInference/kokoro-82m-coreml", "coreml", ["text_to_speech"], [], None, "unknown", "ANE-v3/", None),

    # fp16 component weights only (unet/vae/text_encoders), not the fp32 originals
    # or the combined single-file checkpoint duplicating the same weights.
    ("stabilityai/sdxl-turbo", "mlx", ["image"], [], None, "fp16", None,
     lambda p: p.endswith("fp16.safetensors") and "/" in p),
]

entries = []
errors = []
seen_ids = set()
for repo, fmt, modalities, tags, gguf_file, quant, path_prefix, include in SPECS:
    try:
        entry = pin(repo, fmt, modalities, tags, gguf_file=gguf_file, quant_override=quant,
                    path_prefix=path_prefix, include=include)
        # Two entries can share a repo@rev (whisper small vs turbo); disambiguate id by tag.
        if entry["id"] in seen_ids:
            entry["id"] = f"{entry['id']}#{'-'.join(tags) or path_prefix}"
        seen_ids.add(entry["id"])
        entries.append(entry)
        print(f"OK  {repo} [{path_prefix or '*'}]  ({len(entry['files'])} files, {entry['est_runtime_bytes']/1e9:.2f} GB)", file=sys.stderr)
    except Exception as e:
        errors.append((repo, str(e)))
        print(f"FAIL {repo}: {e}", file=sys.stderr)

if errors:
    print(f"\n{len(errors)} failures, aborting without writing catalog.json", file=sys.stderr)
    sys.exit(1)

catalog = {
    "catalog_version": 1,
    "generated_at": "2026-10-14T00:00:00Z",
    "entries": entries,
}

out_path = os.path.join(here, "..", "crates", "pocket-models", "data", "catalog.json")
with open(out_path, "w") as f:
    json.dump(catalog, f, indent=2)
    f.write("\n")

print(f"\nwrote {len(entries)} entries to {out_path}", file=sys.stderr)
