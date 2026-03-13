#!/usr/bin/env python3
"""
Merge all JSONL training data (original + augmented) into a single
train/eval split for SFT Phase 2 v3.
90/10 split, stratified by source file.
"""
import hashlib, json, random, pathlib, sys

random.seed(42)
BASE = pathlib.Path(__file__).parent.parent
SRC = BASE / "training_data" / "instruction_tuning"
DST = BASE / "training" / "data"
DST.mkdir(parents=True, exist_ok=True)

# All JSONL files to include
files = sorted(SRC.glob("*.jsonl"))
print(f"Found {len(files)} JSONL files in {SRC}")

train_all = []
eval_all = []
canary_all = []


def content_hash(example):
    canonical = json.dumps(example, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def deduplicate_examples(examples):
    seen = set()
    deduped = []
    for example in examples:
        digest = content_hash(example)
        if digest in seen:
            continue
        seen.add(digest)
        deduped.append(example)
    return deduped

for f in files:
    with open(f, "r", encoding="utf-8") as fh:
        lines = [json.loads(l.strip()) for l in fh if l.strip()]

    lines = deduplicate_examples(lines)

    random.shuffle(lines)
    canary_count = min(20, max(0, len(lines) // 20))
    canary = lines[:canary_count]
    remainder = lines[canary_count:]
    split_idx = max(1, int(len(remainder) * 0.9))
    train = remainder[:split_idx]
    eval_ = remainder[split_idx:]

    print(f"  {f.name}: {len(lines)} unique → {len(train)} train + {len(eval_)} eval + {len(canary)} canary")
    train_all.extend(train)
    eval_all.extend(eval_)
    canary_all.extend(canary)

train_all = deduplicate_examples(train_all)
eval_all = deduplicate_examples(eval_all)
canary_all = deduplicate_examples(canary_all)
random.shuffle(train_all)
random.shuffle(eval_all)
random.shuffle(canary_all)

train_path = DST / "sft_train.jsonl"
eval_path = DST / "sft_eval.jsonl"
canary_path = DST / "sft_canary.jsonl"

with open(train_path, "w", encoding="utf-8") as f:
    for ex in train_all:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

with open(eval_path, "w", encoding="utf-8") as f:
    for ex in eval_all:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

with open(canary_path, "w", encoding="utf-8") as f:
    for ex in canary_all[:20]:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

print(f"\n✓ {train_path}: {len(train_all)} examples ({train_path.stat().st_size / 1024 / 1024:.1f} MB)")
print(f"✓ {eval_path}: {len(eval_all)} examples ({eval_path.stat().st_size / 1024 / 1024:.1f} MB)")
print(f"✓ {canary_path}: {min(len(canary_all), 20)} examples ({canary_path.stat().st_size / 1024 / 1024:.1f} MB)")
print(f"Total: {len(train_all) + len(eval_all) + min(len(canary_all), 20)} examples")
