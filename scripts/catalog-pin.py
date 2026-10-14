#!/usr/bin/env python3
"""CORE-06: pin a Hugging Face repo into a ModelManifest-shaped catalog entry.

Never hand-types sizes or hashes: fetches the current commit sha, the file
tree (sizes + LFS sha256), and config.json for context/layer info, straight
from the Hugging Face API.

Usage: catalog-pin.py <repo> --format mlx|gguf|coreml|whisperkit --modalities text,vision
                       --tags recommended,fast [--gguf-file name.gguf]
"""
import argparse
import json
import sys
import urllib.request

API = "https://huggingface.co/api/models"


def fetch_json(url):
    req = urllib.request.Request(url, headers={"User-Agent": "pocketai-catalog-pin/1"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        return json.loads(resp.read())


def fetch_text(url):
    req = urllib.request.Request(url, headers={"User-Agent": "pocketai-catalog-pin/1"})
    with urllib.request.urlopen(req, timeout=30) as resp:
        return resp.read().decode("utf-8", errors="replace")


def pin(repo, fmt, modalities, tags, gguf_file=None, quant_override=None, path_prefix=None, include=None):
    meta = fetch_json(f"{API}/{repo}")
    sha = meta["sha"]
    tree = fetch_json(f"{API}/{repo}/tree/{sha}?recursive=1")

    files = []
    for entry in tree:
        if entry.get("type") != "file":
            continue
        path = entry["path"]
        if path_prefix is not None and not path.startswith(path_prefix):
            continue
        if gguf_file is not None and fmt == "gguf" and not path.endswith(gguf_file):
            continue
        if include is not None and not include(path):
            continue
        size = entry.get("size", 0)
        sha256 = (entry.get("lfs") or {}).get("oid")
        if sha256 is None:
            continue  # not an LFS-tracked weight file; skip (READMEs etc.)
        files.append({
            "path": path,
            "size": size,
            "sha256": sha256,
            "url": f"https://huggingface.co/{repo}/resolve/{sha}/{path}",
        })

    context_length = 4096
    params_b = None
    try:
        config = json.loads(fetch_text(f"https://huggingface.co/{repo}/resolve/{sha}/config.json"))
        context_length = config.get("max_position_embeddings") or config.get("n_positions") or context_length
    except Exception as e:
        print(f"  (no config.json or unreadable for {repo}: {e})", file=sys.stderr)

    license_spdx = (meta.get("cardData") or {}).get("license", "unknown")
    tags_from_hf = meta.get("tags", [])

    total_bytes = sum(f["size"] for f in files)

    entry = {
        "id": f"{repo}@{sha}",
        "display_name": repo.split("/")[-1],
        "family": repo.split("/")[-1].split("-")[0],
        "params_b": params_b,
        "quant": quant_override or ("4bit" if "4bit" in repo.lower() or "4-bit" in repo.lower() else "unknown"),
        "format": fmt,
        "modalities": modalities,
        "context_length": context_length,
        "files": files,
        "license": {
            "spdx": license_spdx,
            "commercial_ok": license_spdx not in ("unknown", "other", "cc-by-nc-4.0"),
            "url": f"https://huggingface.co/{repo}",
        },
        "hf_repo": repo,
        "hf_revision": sha,
        "min_ram_bytes": 0,
        "est_runtime_bytes": int(total_bytes * 1.15),
        "tags": tags,
        "curated": True,
    }
    return entry


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("repo")
    ap.add_argument("--format", required=True, choices=["mlx", "gguf", "coreml", "whisperkit"])
    ap.add_argument("--modalities", required=True)
    ap.add_argument("--tags", default="")
    ap.add_argument("--gguf-file")
    ap.add_argument("--quant")
    args = ap.parse_args()

    entry = pin(
        args.repo,
        args.format,
        args.modalities.split(","),
        [t for t in args.tags.split(",") if t],
        gguf_file=args.gguf_file,
        quant_override=args.quant,
    )
    print(json.dumps(entry, indent=2))


if __name__ == "__main__":
    main()
