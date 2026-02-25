#!/usr/bin/env python3
"""
Merge all JSONL training data (original + augmented) into a single
train/eval split for SFT Phase 2 v3.
90/10 split, stratified by source file.
"""
import json, random, pathlib, sys

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

for f in files:
    with open(f, "r", encoding="utf-8") as fh:
        lines = [json.loads(l.strip()) for l in fh if l.strip()]

    random.shuffle(lines)
    split_idx = max(1, int(len(lines) * 0.9))
    train = lines[:split_idx]
    eval_ = lines[split_idx:]

    print(f"  {f.name}: {len(lines)} total → {len(train)} train + {len(eval_)} eval")
    train_all.extend(train)
    eval_all.extend(eval_)

random.shuffle(train_all)
random.shuffle(eval_all)

train_path = DST / "sft_train.jsonl"
eval_path = DST / "sft_eval.jsonl"

with open(train_path, "w", encoding="utf-8") as f:
    for ex in train_all:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

with open(eval_path, "w", encoding="utf-8") as f:
    for ex in eval_all:
        f.write(json.dumps(ex, ensure_ascii=False) + "\n")

print(f"\n✓ {train_path}: {len(train_all)} examples ({train_path.stat().st_size / 1024 / 1024:.1f} MB)")
print(f"✓ {eval_path}: {len(eval_all)} examples ({eval_path.stat().st_size / 1024 / 1024:.1f} MB)")
print(f"Total: {len(train_all) + len(eval_all)} examples")
