#!/usr/bin/env python3
"""Download NER / embedding model weights from HuggingFace for ApexIntel entity extraction pipeline."""
import os
import sys

try:
    from huggingface_hub import snapshot_download
except ImportError:
    print("Installing huggingface_hub...")
    os.system(f"{sys.executable} -m pip install -q huggingface_hub")
    from huggingface_hub import snapshot_download

DEST = os.path.dirname(os.path.abspath(__file__))

# Models needed per IMPLEMENTATION.md Section 8.0:
# 1. NER models for entity extraction pipeline
# 2. Embedding model for entity resolution / dedup
# 3. Relation extraction model for knowledge graph
MODELS = [
    {
        "repo": "dslim/bert-base-NER",
        "dir": "bert-base-NER",
        "desc": "English NER (PER, ORG, LOC, MISC) - BERT base",
        "size_mb": 420,
        # Only need config, tokenizer, and model weights (skip tf/flax/onnx)
        "allow": ["*.json", "*.txt", "*.bin", "*.safetensors", "tokenizer*"],
        "ignore": ["*tf*", "*flax*", "*.ot", "*.msgpack", "onnx/*", "onnx/**"],
    },
    {
        "repo": "Jean-Baptiste/camembert-ner",
        "dir": "camembert-ner",
        "desc": "French NER (CamemBERT-based)",
        "size_mb": 420,
        "allow": ["*.json", "*.txt", "*.bin", "*.safetensors", "tokenizer*", "sentencepiece*"],
        "ignore": ["*tf*", "*flax*", "*.ot", "*.msgpack", "onnx/*", "onnx/**"],
    },
    {
        "repo": "Davlan/xlm-roberta-large-ner-hrl",
        "dir": "xlm-roberta-large-ner-hrl",
        "desc": "Multilingual NER (ar, de, en, es, fr, it, nl, pt, zh) - XLM-R large",
        "size_mb": 1400,
        "allow": ["*.json", "*.txt", "*.bin", "*.safetensors", "tokenizer*", "sentencepiece*"],
        "ignore": ["*tf*", "*flax*", "*.ot", "*.msgpack", "onnx/*", "onnx/**"],
    },
    {
        "repo": "Babelscape/rebel-large",
        "dir": "rebel-large",
        "desc": "Relation Extraction (triplet extraction for knowledge graph)",
        "size_mb": 800,
        "allow": ["*.json", "*.txt", "*.bin", "*.safetensors", "tokenizer*", "sentencepiece*"],
        "ignore": ["*tf*", "*flax*", "*.ot", "*.msgpack", "onnx/*", "onnx/**"],
    },
    {
        "repo": "sentence-transformers/all-MiniLM-L6-v2",
        "dir": "all-MiniLM-L6-v2",
        "desc": "Sentence embeddings for entity resolution & semantic dedup",
        "size_mb": 80,
        "allow": ["*.json", "*.txt", "*.bin", "*.safetensors", "tokenizer*", "modules.json", "config_sentence_transformers.json"],
        "ignore": ["*tf*", "*flax*", "*.ot", "*.msgpack", "onnx/*", "onnx/**"],
    },
]


def download_model(model_info):
    repo = model_info["repo"]
    local = os.path.join(DEST, model_info["dir"])
    desc = model_info["desc"]
    est = model_info["size_mb"]

    if os.path.exists(local) and len(os.listdir(local)) > 2:
        total = sum(
            os.path.getsize(os.path.join(local, f))
            for f in os.listdir(local)
            if os.path.isfile(os.path.join(local, f))
        )
        print(f"  SKIP {repo} — already downloaded ({total / 1e6:.1f} MB)")
        return True

    print(f"  Downloading {repo} (~{est} MB) — {desc}")
    try:
        snapshot_download(
            repo_id=repo,
            local_dir=local,
            allow_patterns=model_info.get("allow"),
            ignore_patterns=model_info.get("ignore"),
        )
        total = 0
        for root, dirs, files in os.walk(local):
            for f in files:
                total += os.path.getsize(os.path.join(root, f))
        print(f"    ✓ {total / 1e6:.1f} MB")
        return True
    except Exception as e:
        print(f"    ✗ Error: {e}")
        return False


def main():
    print(f"Downloading NER/embedding model weights to {DEST}/\n")
    success = 0
    for m in MODELS:
        if download_model(m):
            success += 1
    print(f"\n{success}/{len(MODELS)} models downloaded.")

    # Summary
    total_size = 0
    for m in MODELS:
        d = os.path.join(DEST, m["dir"])
        if os.path.isdir(d):
            for root, dirs, files in os.walk(d):
                for f in files:
                    total_size += os.path.getsize(os.path.join(root, f))
    print(f"Total model weights: {total_size / 1e9:.2f} GB")


if __name__ == "__main__":
    main()
