#!/usr/bin/env python3
"""Check training data field names vs eval expected_schema."""
import json, os, re
from pathlib import Path

WORK = Path(__file__).resolve().parent.parent

TASK_KEYWORDS = {
    "company_dossier": ["company dossier", "company_dossier", "dossier"],
    "memo_quality": ["weekly memo", "memo_quality", "intelligence memo", "weekly intelligence"],
    "warning_generation": ["warning", "warning_generation", "generate a warning", "generate warning"],
    "entity_extraction": ["entity extraction", "entity_extraction", "extract entities", "extract the entities"],
}

EVAL_EXPECTED = {
    "company_dossier": ["profile_section", "capability_assessment", "certification_analysis", "risk_assessment", "opportunity_analysis", "competitive_position"],
    "memo_quality": ["executive_summary", "total_insights", "critical_count", "warning_count", "info_count", "top_actions", "regional_sections", "security_summary"],
    "warning_generation": ["warning_type", "severity", "affected_entity", "narrative", "recommended_actions"],
    "entity_extraction": ["companies", "persons", "capabilities", "certifications"],
}

data_path = os.environ.get("TRAIN_FILE", str(WORK / "training" / "data" / "sft_train.jsonl"))

with open(data_path) as f:
    lines = f.readlines()

print(f"Total training examples: {len(lines)}")
print()

for task, keywords in TASK_KEYWORDS.items():
    print(f"=== {task} ===")
    print(f"  Eval expected fields: {EVAL_EXPECTED[task]}")
    found = 0
    for line in lines:
        item = json.loads(line)
        msgs = item.get("messages", [])
        # Check if system or user message mentions this task
        text = " ".join(m.get("content", "") for m in msgs if m["role"] in ("system", "user")).lower()
        if any(kw in text for kw in keywords):
            # Get assistant response
            for m in msgs:
                if m["role"] == "assistant":
                    content = m["content"]
                    try:
                        obj = json.loads(content)
                        if isinstance(obj, dict):
                            keys = sorted(obj.keys())
                            if found < 3:
                                print(f"  Training example {found+1} keys: {keys}")
                                # Check overlap with eval expected
                                expected = set(EVAL_EXPECTED[task])
                                actual = set(keys)
                                overlap = expected & actual
                                missing = expected - actual
                                extra = actual - expected
                                print(f"    Match: {len(overlap)}/{len(expected)} — {overlap}")
                                if missing:
                                    print(f"    Missing from eval schema: {missing}")
                                if extra:
                                    print(f"    Extra (not in eval): {extra}")
                            found += 1
                    except json.JSONDecodeError:
                        # Try to find JSON in content
                        match = re.search(r'\{[\s\S]*\}', content)
                        if match:
                            try:
                                obj = json.loads(match.group())
                                if isinstance(obj, dict) and found < 3:
                                    keys = sorted(obj.keys())
                                    print(f"  Training example {found+1} keys (embedded): {keys}")
                                    found += 1
                            except:
                                pass
                    break
    print(f"  Total found: {found}")
    print()

# Also check severity values in warning_generation
print("=== warning_generation severity values ===")
for line in lines:
    item = json.loads(line)
    msgs = item.get("messages", [])
    text = " ".join(m.get("content", "") for m in msgs if m["role"] in ("system", "user")).lower()
    if any(kw in text for kw in TASK_KEYWORDS["warning_generation"]):
        for m in msgs:
            if m["role"] == "assistant":
                try:
                    obj = json.loads(m["content"])
                    if isinstance(obj, dict) and "severity" in obj:
                        sev = obj["severity"]
                        print(f"  severity: {sev}")
                except:
                    pass
                break
