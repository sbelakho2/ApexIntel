#!/usr/bin/env python3
"""Compare eval prompts vs training prompts to find mismatches."""
import json
import os
import sys

eval_dir = "/workspace/ApexIntel/training_data/evaluation"
train_file = "/workspace/ApexIntel/training/data/sft_train.jsonl"

tasks = ["company_dossier", "warning_generation", "memo_quality",
         "poi_synthesis", "competitive_analysis", "recipe_quality"]

# 1. Get eval prompts
print("=" * 80)
print("EVAL PROMPTS (system + user first 300 chars)")
print("=" * 80)

eval_prompts = {}
for task in tasks:
    fp = os.path.join(eval_dir, f"{task}_eval.jsonl")
    try:
        with open(fp) as f:
            item = json.loads(f.readline())
        inp = item.get("input", {})
        if isinstance(inp, dict):
            sys_msg = inp.get("system", "NONE")
            user_msg = inp.get("user", "NONE")
        else:
            sys_msg = "NONE"
            user_msg = str(inp)[:300]
        eval_prompts[task] = {"system": sys_msg, "user": user_msg}
        print(f"\n--- {task} ---")
        print(f"  SYSTEM: {sys_msg[:300]}")
        print(f"  USER: {user_msg[:300]}")
    except Exception as e:
        print(f"\n--- {task} --- ERROR: {e}")

# 2. Get training prompts (first match per task)
print("\n\n" + "=" * 80)
print("TRAINING PROMPTS (system + user first 300 chars)")
print("=" * 80)

task_examples = {}
with open(train_file) as f:
    for line in f:
        item = json.loads(line)
        msgs = item.get("messages", [])
        if len(msgs) < 3:
            continue
        sys_content = msgs[0].get("content", "") if msgs[0]["role"] == "system" else ""
        user_content = msgs[1].get("content", "") if msgs[1]["role"] == "user" else ""
        for task in tasks:
            key = task.replace("_", " ")
            if key in sys_content.lower() or key in user_content.lower()[:200]:
                if task not in task_examples:
                    task_examples[task] = {
                        "system": sys_content,
                        "user": user_content,
                        "assistant_keys": []
                    }
                    # Also get assistant response keys
                    asst = msgs[2].get("content", "") if msgs[2]["role"] == "assistant" else ""
                    try:
                        parsed = json.loads(asst)
                        if isinstance(parsed, dict):
                            task_examples[task]["assistant_keys"] = list(parsed.keys())
                    except:
                        task_examples[task]["assistant_keys"] = ["NOT_JSON"]

for task in tasks:
    if task in task_examples:
        ex = task_examples[task]
        print(f"\n--- {task} ---")
        print(f"  SYSTEM: {ex['system'][:300]}")
        print(f"  USER: {ex['user'][:300]}")
        print(f"  ASSISTANT KEYS: {ex['assistant_keys']}")
    else:
        print(f"\n--- {task} --- NOT FOUND in training data")

# 3. Show what eval expects
print("\n\n" + "=" * 80)
print("EVAL EXPECTED SCHEMAS (required_fields)")
print("=" * 80)
for task in tasks:
    fp = os.path.join(eval_dir, f"{task}_eval.jsonl")
    try:
        with open(fp) as f:
            item = json.loads(f.readline())
        schema = item.get("expected_schema", {})
        req = schema.get("required_fields", [])
        print(f"\n{task}: {req}")
    except Exception as e:
        print(f"\n{task}: ERROR {e}")

# 4. Show actual model output for company_dossier from last eval
print("\n\n" + "=" * 80)
print("EVAL REPORT - SAMPLE FAILURES")
print("=" * 80)
try:
    with open("/workspace/ApexIntel/training/outputs/eval_report.json") as f:
        report = json.load(f)
    for result in report.get("results", []):
        if not result.get("passed") and result.get("task") in tasks:
            task = result["task"]
            raw = result.get("raw_output", "")
            print(f"\n--- {task} (id: {result.get('id', '?')}) ---")
            print(f"  DETAILS: {result.get('details', {})}")
            print(f"  RAW OUTPUT (first 400): {raw[:400]}")
            if task not in ["__shown"]:
                pass
except Exception as e:
    print(f"Could not read eval report: {e}")
