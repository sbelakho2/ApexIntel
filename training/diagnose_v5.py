#!/usr/bin/env python3
"""Diagnose all v5 eval failures by comparing model output to expected data."""
import json
from pathlib import Path

EVAL_DIR = Path("/workspace/ApexIntel/training_data/evaluation")
REPORT = Path("/workspace/ApexIntel/training/outputs/eval_report.json")

# Load failure report
report = json.load(open(REPORT))
failures = report.get("failures", [])

print(f"=== v5 FAILURES: {len(failures)} ===\n")

# Load all eval data indexed by ID
all_items = {}
for p in EVAL_DIR.glob("*.jsonl"):
    with open(p) as f:
        for line in f:
            if not line.strip():
                continue
            item = json.loads(line)
            all_items[item.get("id", "")] = item

for fail in failures:
    fid = fail["id"]
    ftype = fail["type"]
    metrics = fail.get("metrics", {})
    preview = fail.get("output_preview", "(none)")
    
    item = all_items.get(fid, {})
    
    print(f"--- {ftype} | {fid[:12]} ---")
    print(f"  Metrics: {json.dumps(metrics)}")
    
    if ftype in ("adversarial_tests", "adversarial"):
        mnc = item.get("must_not_contain", [])
        mbvj = item.get("must_be_valid_json")
        ee = item.get("expected_entities")
        print(f"  must_not_contain: {mnc}")
        print(f"  must_be_valid_json: {mbvj}")
        print(f"  has expected_entities: {ee is not None}")
        # Check which must_not_contain items are IN the output
        lower_out = preview.lower()
        for bad in mnc or []:
            if bad.lower() in lower_out:
                print(f"  >> FOUND forbidden string: '{bad}'")
        if ee:
            print(f"  expected_entities keys: {list(ee.keys())}")
    
    elif ftype == "compliance":
        es = item.get("expected_schema", {})
        rf = es.get("required_fields", [])
        print(f"  required_fields: {rf}")
        print(f"  expected_risk_level: {item.get('expected_risk_level')}")
        # Try to parse the JSON from output
        try:
            import re
            clean = preview
            # Strip think tags
            clean = re.sub(r"<think>.*?</think>", "", clean, flags=re.DOTALL)
            clean = re.sub(r"<think>.*$", "", clean, flags=re.DOTALL)
            # Find JSON
            start = clean.find("{")
            if start >= 0:
                # Try to find matching brace
                depth = 0
                for i, c in enumerate(clean[start:]):
                    if c == "{": depth += 1
                    elif c == "}": depth -= 1
                    if depth == 0:
                        json_str = clean[start:start+i+1]
                        obj = json.loads(json_str)
                        print(f"  Parsed JSON keys: {list(obj.keys())[:10]}")
                        present = [k for k in rf if k in obj]
                        missing = [k for k in rf if k not in obj]
                        print(f"  Present fields: {present}")
                        print(f"  Missing fields: {missing}")
                        break
                else:
                    print(f"  JSON is INCOMPLETE (unclosed braces)")
                    print(f"  Output ends with: ...{preview[-100:]}")
            else:
                print(f"  No JSON found in output")
        except json.JSONDecodeError as e:
            print(f"  JSON parse error: {e}")
    
    elif ftype in ("poi_synthesis",):
        es = item.get("expected_schema", {})
        rf = es.get("required_fields", [])
        print(f"  required_fields: {rf}")
        # Check what model produced
        try:
            import re
            clean = preview
            clean = re.sub(r"<think>.*?</think>", "", clean, flags=re.DOTALL)
            clean = re.sub(r"<think>.*$", "", clean, flags=re.DOTALL)
            start = clean.find("{")
            if start >= 0:
                depth = 0
                for i, c in enumerate(clean[start:]):
                    if c == "{": depth += 1
                    elif c == "}": depth -= 1
                    if depth == 0:
                        obj = json.loads(clean[start:start+i+1])
                        print(f"  Model output keys: {list(obj.keys())[:15]}")
                        present = [k for k in rf if k in obj]
                        missing = [k for k in rf if k not in obj]
                        print(f"  Present: {present}")
                        print(f"  Missing: {missing}")
                        print(f"  Coverage: {len(present)}/{len(rf)} = {len(present)/max(len(rf),1):.2f}")
                        break
                else:
                    print(f"  JSON INCOMPLETE")
        except Exception as e:
            print(f"  Parse error: {e}")
    
    elif ftype == "entity_extraction" or ftype in ("regression_tests", "multilingual_golden"):
        gt = item.get("ground_truth", item.get("expected_entities", item.get("expected", {})))
        # Collect gold entities
        import re
        def normalize(s):
            s = s.lower().strip()
            s = re.sub(r"\s+", " ", s)
            s = re.sub(r"\s*\b(inc\.?|corp\.?|ltd\.?|llc|co\.?|gmbh|s\.?a\.?|plc|limited|incorporated|corporation|company)\s*$", "", s)
            return s.rstrip(" ,.")
        
        def collect(obj):
            collected = []
            for e in obj.get("companies", []) or []:
                if isinstance(e, dict): collected.append(f"company:{normalize(e.get('name',''))}")
                else: collected.append(f"company:{normalize(str(e))}")
            for e in obj.get("persons", []) or []:
                if isinstance(e, dict): collected.append(f"person:{normalize(e.get('name',''))}")
                else: collected.append(f"person:{normalize(str(e))}")
            for e in obj.get("capabilities", []) or []:
                collected.append(f"capability:{normalize(str(e))}")
            for e in obj.get("certifications", []) or []:
                collected.append(f"cert:{normalize(str(e))}")
            for e in obj.get("locations", []) or []:
                if isinstance(e, dict):
                    for k in ("city", "country", "state", "prefecture", "name"):
                        if k in e: collected.append(f"loc:{normalize(str(e[k]))}")
                else:
                    collected.append(f"loc:{normalize(str(e))}")
            for e in obj.get("industries", []) or []:
                if isinstance(e, str): collected.append(f"industry:{normalize(e)}")
                elif isinstance(e, dict): collected.append(f"industry:{normalize(e.get('name', str(e)))}")
            return collected
        
        gold = collect(gt)
        print(f"  Gold entities ({len(gold)}): {gold}")
        
        # Try to parse model output
        try:
            clean = preview
            clean = re.sub(r"<think>.*?</think>", "", clean, flags=re.DOTALL)
            clean = re.sub(r"<think>.*$", "", clean, flags=re.DOTALL)
            start = clean.find("{")
            if start >= 0:
                depth = 0
                for i, c in enumerate(clean[start:]):
                    if c == "{": depth += 1
                    elif c == "}": depth -= 1
                    if depth == 0:
                        obj = json.loads(clean[start:start+i+1])
                        pred = collect(obj)
                        print(f"  Pred entities ({len(pred)}): {pred}")
                        # Show matches
                        matched_gold = set()
                        matched_pred = set()
                        for p in pred:
                            for g in gold:
                                if p == g or p.split(":",1)[-1] == g.split(":",1)[-1]:
                                    matched_pred.add(p)
                                    matched_gold.add(g)
                                elif len(p.split(":",1)[-1]) >= 3 and len(g.split(":",1)[-1]) >= 3:
                                    if p.split(":",1)[-1] in g.split(":",1)[-1] or g.split(":",1)[-1] in p.split(":",1)[-1]:
                                        matched_pred.add(p)
                                        matched_gold.add(g)
                        unmatched_gold = [g for g in gold if g not in matched_gold]
                        unmatched_pred = [p for p in pred if p not in matched_pred]
                        print(f"  Unmatched gold: {unmatched_gold}")
                        print(f"  Unmatched pred: {unmatched_pred}")
                        break
                else:
                    print(f"  JSON INCOMPLETE in preview")
        except Exception as e:
            print(f"  Parse error: {e}")
    
    print()
