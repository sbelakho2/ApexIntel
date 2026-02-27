#!/usr/bin/env python3
"""
Fix eval JSONL files: replace shortened system prompts with the full training system prompts.
This ensures the model sees the same instructions at eval time that it learned from.
"""
import json
import os
import sys
from collections import defaultdict

eval_dir = "/workspace/ApexIntel/training_data/evaluation"
train_file = "/workspace/ApexIntel/training/data/sft_train.jsonl"

# Step 1: Extract the FULL system prompts from training data for each task type
# We match by looking at assistant response keys to identify task type
task_fields = {
    "company_dossier": {"profile_section", "capability_assessment", "certification_analysis", "risk_assessment", "opportunity_analysis", "competitive_position"},
    "warning_generation": {"warning_type", "severity", "affected_entity", "narrative", "recommended_actions"},
    "memo_quality": {"executive_summary", "total_insights", "critical_count", "warning_count", "info_count", "top_actions", "regional_sections", "security_summary"},
    "poi_synthesis": {"person_id", "name", "org", "current_role", "role_family", "priority_vector", "psychological_profile", "influence_assessment", "what_changed", "what_it_implies", "how_to_approach"},
    "recipe_quality": {"id", "code", "name", "description", "signals", "transforms", "statistical_test", "insight_template", "action_template", "applicability", "severity", "category"},
    "competitive_analysis": {"capability_comparison", "certification_comparison", "scale_comparison", "advantages", "gaps", "recommendations"},
    "supply_chain_risk": {"risk_summary", "affected_components", "severity", "mitigation_options"},
    "compliance": {"risk_level"},  # Simplified for matching
}

# Collect FIRST LONGEST system prompt per task from training data
task_system_prompts = {}
with open(train_file) as f:
    for line in f:
        item = json.loads(line)
        msgs = item.get("messages", [])
        if len(msgs) < 3:
            continue

        sys_msg = msgs[0]["content"] if msgs[0]["role"] == "system" else ""
        asst_msg = msgs[2]["content"] if msgs[2]["role"] == "assistant" else ""

        try:
            parsed = json.loads(asst_msg)
            if not isinstance(parsed, dict):
                continue
            keys = set(parsed.keys())
        except:
            continue

        for task, required in task_fields.items():
            overlap = keys & required
            coverage = len(overlap) / len(required) if required else 0
            if coverage >= 0.7:
                if task not in task_system_prompts or len(sys_msg) > len(task_system_prompts[task]):
                    task_system_prompts[task] = sys_msg

print("Extracted system prompts from training data:")
for task, prompt in sorted(task_system_prompts.items()):
    print(f"\n  {task} ({len(prompt)} chars): {prompt[:150]}...")

# Step 2: Update each eval JSONL file
eval_files = sorted([f for f in os.listdir(eval_dir) if f.endswith("_eval.jsonl")])
print(f"\nFound {len(eval_files)} eval files")

for eval_file in eval_files:
    task_name = eval_file.replace("_eval.jsonl", "")
    filepath = os.path.join(eval_dir, eval_file)

    # Read all lines
    with open(filepath) as f:
        lines = [json.loads(l) for l in f if l.strip()]

    if not lines:
        print(f"\n  {eval_file}: EMPTY, skipping")
        continue

    # Check if this task has a training system prompt
    if task_name in task_system_prompts:
        new_sys = task_system_prompts[task_name]
        old_sys = lines[0].get("input", {}).get("system", "NONE") if isinstance(lines[0].get("input"), dict) else "NONE"

        if old_sys == new_sys:
            print(f"\n  {eval_file}: ALREADY MATCHES training prompt")
            continue

        # Update all examples
        updated = 0
        for item in lines:
            if isinstance(item.get("input"), dict) and "system" in item["input"]:
                item["input"]["system"] = new_sys
                updated += 1

        # Write back
        with open(filepath, "w") as f:
            for item in lines:
                f.write(json.dumps(item, ensure_ascii=False) + "\n")

        print(f"\n  {eval_file}: UPDATED {updated} examples")
        print(f"    OLD: {old_sys[:100]}...")
        print(f"    NEW: {new_sys[:100]}...")

    else:
        old_sys = "N/A"
        if lines and isinstance(lines[0].get("input"), dict):
            old_sys = lines[0]["input"].get("system", "N/A")[:100]
        print(f"\n  {eval_file}: NO training match found (task={task_name})")
        print(f"    Current system: {old_sys}")

print("\n\nDone! Eval files updated with full training system prompts.")
