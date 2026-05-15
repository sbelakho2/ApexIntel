#!/usr/bin/env python3
"""Check expected_schema and scoring for all eval files."""
import json, os, glob
from pathlib import Path

WORK = Path(__file__).resolve().parent.parent
eval_dir = os.environ.get("EVAL_DIR", str(WORK / "training_data" / "evaluation"))
for path in sorted(glob.glob(os.path.join(eval_dir, "*.jsonl"))):
    name = os.path.basename(path)
    with open(path) as f:
        item = json.loads(f.readline())
    print(f"=== {name} ===")
    print(f"  Top keys: {list(item.keys())}")
    es = item.get("expected_schema", {})
    if es:
        print(f"  expected_schema: {json.dumps(es)}")
    gt = item.get("ground_truth", item.get("expected_entities", {}))
    if gt:
        if isinstance(gt, dict):
            print(f"  ground_truth keys: {list(gt.keys())}")
            for k, v in gt.items():
                if isinstance(v, list) and len(v) > 0:
                    print(f"    {k}: {len(v)} items, first={v[0]}")
                else:
                    print(f"    {k}: {v}")
        else:
            print(f"  ground_truth: {str(gt)[:200]}")
    ee = item.get("expected", {})
    if ee:
        print(f"  expected: {json.dumps(ee) if isinstance(ee,dict) else str(ee)[:200]}")
    scoring = item.get("scoring", {})
    if scoring:
        print(f"  scoring: {json.dumps(scoring)}")
    # Check for other relevant keys
    for k in ["must_not_contain", "expected_severity", "expected_entities"]:
        v = item.get(k)
        if v:
            print(f"  {k}: {json.dumps(v) if isinstance(v,(dict,list)) else v}")
    print()
