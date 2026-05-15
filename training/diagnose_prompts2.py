#!/usr/bin/env python3
"""Find what system prompts training data uses for examples with correct output schemas."""
import json
import os
from pathlib import Path

WORK = Path(__file__).resolve().parent.parent
eval_dir = os.environ.get("EVAL_DIR", str(WORK / "training_data" / "evaluation"))
train_file = os.environ.get("TRAIN_FILE", str(WORK / "training" / "data" / "sft_train.jsonl"))

# Required fields per task from eval
task_fields = {
    "company_dossier": {"profile_section", "capability_assessment", "certification_analysis", "risk_assessment", "opportunity_analysis", "competitive_position"},
    "warning_generation": {"warning_type", "severity", "affected_entity", "narrative", "recommended_actions"},
    "memo_quality": {"executive_summary", "total_insights", "critical_count", "warning_count", "info_count", "top_actions", "regional_sections", "security_summary"},
    "poi_synthesis": {"person_id", "name", "org", "current_role", "role_family", "priority_vector", "psychological_profile", "influence_assessment", "what_changed", "what_it_implies", "how_to_approach"},
    "recipe_quality": {"id", "code", "name", "description", "signals", "transforms", "statistical_test", "insight_template", "action_template", "applicability", "severity", "category"},
    "competitive_analysis": {"capability_comparison", "certification_comparison", "scale_comparison", "advantages", "gaps", "recommendations"},
}

# Classify every training example by which task's schema it matches
task_system_prompts = {t: set() for t in task_fields}
task_user_samples = {t: [] for t in task_fields}

line_count = 0
with open(train_file) as f:
    for line in f:
        line_count += 1
        item = json.loads(line)
        msgs = item.get("messages", [])
        if len(msgs) < 3:
            continue
        
        sys_msg = msgs[0]["content"] if msgs[0]["role"] == "system" else ""
        user_msg = msgs[1]["content"] if msgs[1]["role"] == "user" else ""
        asst_msg = msgs[2]["content"] if msgs[2]["role"] == "assistant" else ""
        
        # Parse assistant JSON
        try:
            parsed = json.loads(asst_msg)
            if not isinstance(parsed, dict):
                continue
            keys = set(parsed.keys())
        except:
            continue
        
        # Match against each task's required fields
        for task, required in task_fields.items():
            overlap = keys & required
            coverage = len(overlap) / len(required) if required else 0
            if coverage >= 0.7:  # Same threshold as eval
                task_system_prompts[task].add(sys_msg[:200])  # Deduplicate unique system prompts
                if len(task_user_samples[task]) < 2:
                    task_user_samples[task].append({
                        "line": line_count,
                        "system": sys_msg[:200],
                        "user_prefix": user_msg[:200],
                        "asst_keys": sorted(keys)[:15],
                        "coverage": f"{len(overlap)}/{len(required)}"
                    })

print(f"Total training lines: {line_count}")
print()
for task in task_fields:
    prompts = task_system_prompts[task]
    print(f"\n{'='*70}")
    print(f"TASK: {task} ({len(prompts)} unique system prompts)")
    print(f"{'='*70}")
    for i, p in enumerate(prompts):
        print(f"  System Prompt #{i+1}: {p}")
    for s in task_user_samples[task]:
        print(f"\n  Sample (line {s['line']}):")
        print(f"    SYSTEM: {s['system']}")
        print(f"    USER: {s['user_prefix']}")
        print(f"    ASST KEYS: {s['asst_keys']}")
        print(f"    COVERAGE: {s['coverage']}")
