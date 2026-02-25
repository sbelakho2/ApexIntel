#!/usr/bin/env python3
"""
validate_pipeline.py — Pre-flight checks for the ApexIntel training pipeline.

Verifies all configs, data files, scripts, and dependencies are present and
internally consistent before committing GPU hours.

Usage:
    python training/validate_pipeline.py [--work-dir /workspace/ApexIntel] [--strict]
"""

import argparse
import importlib
import json
import os
import sys
import yaml
from pathlib import Path

PASS = "\033[92m✓\033[0m"
FAIL = "\033[91m✗\033[0m"
WARN = "\033[93m⚠\033[0m"

errors: list[str] = []
warnings: list[str] = []


def ok(msg: str):
    print(f"  {PASS} {msg}")


def fail(msg: str, strict: bool = True):
    print(f"  {FAIL} {msg}")
    if strict:
        errors.append(msg)


def warn(msg: str):
    print(f"  {WARN} {msg}")
    warnings.append(msg)


# ═══════════════════════════════════════════════════════════════════
# 1. File existence checks
# ═══════════════════════════════════════════════════════════════════

REQUIRED_FILES = [
    "training/train_phase1_dapt.py",
    "training/train_phase2_sft.py",
    "training/eval_harness.py",
    "training/prepare_data.py",
    "training/export_model.py",
    "training/run_all.sh",
    "training/setup_vast.sh",
    "training/requirements.txt",
    "training/configs/phase1_dapt.yaml",
    "training/configs/phase2_sft.yaml",
    "training/configs/accelerate_8gpu.yaml",
    "training/configs/ds_zero3_5090.json",
]

REQUIRED_EVAL_FILES = [
    "training_data/evaluation/generate_eval_data.py",
    "training_data/evaluation/recipe_quality_eval.jsonl",
    "training_data/evaluation/poi_synthesis_eval.jsonl",
    "training_data/evaluation/memo_quality_eval.jsonl",
    "training_data/evaluation/entity_extraction_eval.jsonl",
    "training_data/evaluation/regression_tests.jsonl",
    "training_data/evaluation/adversarial_tests.jsonl",
    "training_data/evaluation/multilingual_golden.jsonl",
]


def check_files(work: str, strict: bool):
    print("\n[1/7] Required files")
    for rel in REQUIRED_FILES:
        p = os.path.join(work, rel)
        if os.path.isfile(p):
            ok(rel)
        else:
            fail(f"MISSING: {rel}", strict)

    print("\n[2/7] Evaluation data files")
    for rel in REQUIRED_EVAL_FILES:
        p = os.path.join(work, rel)
        if os.path.isfile(p):
            # Count lines
            with open(p) as f:
                n = sum(1 for _ in f)
            ok(f"{rel}  ({n} examples)")
        else:
            fail(f"MISSING: {rel}", strict)


# ═══════════════════════════════════════════════════════════════════
# 2. Config consistency
# ═══════════════════════════════════════════════════════════════════

def check_configs(work: str, strict: bool):
    print("\n[3/7] Config consistency")

    # Phase 1 config
    p1_path = os.path.join(work, "training/configs/phase1_dapt.yaml")
    p2_path = os.path.join(work, "training/configs/phase2_sft.yaml")
    ds_path = os.path.join(work, "training/configs/ds_zero3_5090.json")
    acc_path = os.path.join(work, "training/configs/accelerate_8gpu.yaml")

    if not all(os.path.isfile(p) for p in [p1_path, p2_path, ds_path, acc_path]):
        fail("Cannot check configs — missing files", strict)
        return

    with open(p1_path) as f:
        p1 = yaml.safe_load(f)
    with open(p2_path) as f:
        p2 = yaml.safe_load(f)
    with open(ds_path) as f:
        # ds_zero3_b200.json has comments (JSONC), strip them
        content = f.read()
        lines = [l for l in content.split("\n") if not l.strip().startswith("//") and not l.strip().startswith("#")]
        ds = json.loads("\n".join(lines))
    with open(acc_path) as f:
        acc = yaml.safe_load(f)

    # Check model paths match
    if p1["model"]["name_or_path"] == p2["model"]["name_or_path"]:
        ok(f"Model path consistent: {p1['model']['name_or_path']}")
    else:
        fail(f"Model path mismatch: P1={p1['model']['name_or_path']} P2={p2['model']['name_or_path']}", strict)

    # Phase 2 references Phase 1 adapter
    p1_out = p1["training"]["output_dir"]
    p2_adapter = p2["model"].get("phase1_adapter", "")
    if p1_out in p2_adapter or "phase1" in p2_adapter:
        ok(f"Phase 2 adapter path references Phase 1 output")
    else:
        fail(f"Phase 2 adapter path ({p2_adapter}) doesn't reference Phase 1 output ({p1_out})", strict)

    # Check DeepSpeed config referenced in both phases
    for name, cfg in [("Phase 1", p1), ("Phase 2", p2)]:
        ds_ref = cfg["training"].get("deepspeed")
        if ds_ref:
            ds_full = os.path.join(work, ds_ref)
            if os.path.isfile(ds_full):
                ok(f"{name} DeepSpeed config exists: {ds_ref}")
            else:
                fail(f"{name} DeepSpeed config not found: {ds_full}", strict)

    # Check bf16 consistency
    if ds.get("bf16", {}).get("enabled") and p1["training"].get("bf16") and p2["training"].get("bf16"):
        ok("bf16 enabled consistently across all configs")
    else:
        warn("bf16 settings may be inconsistent")

    # Check accelerate num_processes
    num_proc = acc.get("num_processes", 0)
    if num_proc >= 4:
        ok(f"Accelerate configured for {num_proc} processes")
    else:
        warn(f"Accelerate only has {num_proc} processes (expected 4+)")

    # ZeRO stage 3 check
    zero_stage = ds.get("zero_optimization", {}).get("stage")
    if zero_stage == 3:
        ok("DeepSpeed ZeRO Stage 3 configured")
    else:
        warn(f"DeepSpeed ZeRO stage is {zero_stage}, expected 3")

    # Check data files referenced in configs
    for name, cfg in [("Phase 1", p1), ("Phase 2", p2)]:
        train_file = os.path.join(work, cfg["data"]["train_file"])
        eval_file = os.path.join(work, cfg["data"]["eval_file"])
        if os.path.isfile(train_file):
            ok(f"{name} train file exists: {cfg['data']['train_file']}")
        else:
            warn(f"{name} train file not yet generated: {cfg['data']['train_file']} (will be created by prepare_data.py)")
        if os.path.isfile(eval_file):
            ok(f"{name} eval file exists: {cfg['data']['eval_file']}")
        else:
            warn(f"{name} eval file not yet generated: {cfg['data']['eval_file']} (will be created by prepare_data.py)")

    # Effective batch size
    for name, cfg in [("Phase 1", p1), ("Phase 2", p2)]:
        tc = cfg["training"]
        micro = tc["per_device_train_batch_size"]
        grad_acc = tc["gradient_accumulation_steps"]
        effective = micro * num_proc * grad_acc
        ok(f"{name} effective batch size: {micro}×{num_proc}×{grad_acc} = {effective}")


# ═══════════════════════════════════════════════════════════════════
# 3. Python dependencies
# ═══════════════════════════════════════════════════════════════════

REQUIRED_PACKAGES = [
    ("torch", "torch"),
    ("transformers", "transformers"),
    ("peft", "peft"),
    ("trl", "trl"),
    ("datasets", "datasets"),
    ("accelerate", "accelerate"),
    ("yaml", "PyYAML"),
]

OPTIONAL_PACKAGES = [
    ("deepspeed", "deepspeed"),
    ("wandb", "wandb"),
    ("gguf", "gguf"),
    ("flash_attn", "flash-attn"),
    ("bitsandbytes", "bitsandbytes"),
    ("rouge_score", "rouge-score"),
    ("sklearn", "scikit-learn"),
    ("rapidfuzz", "rapidfuzz"),
    ("langdetect", "langdetect"),
    ("nltk", "nltk"),
]


def check_dependencies(strict: bool):
    print("\n[4/7] Python dependencies")
    for module_name, pip_name in REQUIRED_PACKAGES:
        try:
            mod = importlib.import_module(module_name)
            ver = getattr(mod, "__version__", "?")
            ok(f"{pip_name} ({ver})")
        except ImportError:
            fail(f"MISSING: {pip_name}  →  pip install {pip_name}", strict)

    for module_name, pip_name in OPTIONAL_PACKAGES:
        try:
            mod = importlib.import_module(module_name)
            ver = getattr(mod, "__version__", "?")
            ok(f"{pip_name} ({ver})")
        except ImportError:
            warn(f"Optional: {pip_name} not installed  →  pip install {pip_name}")


# ═══════════════════════════════════════════════════════════════════
# 4. GPU checks
# ═══════════════════════════════════════════════════════════════════

def check_gpu():
    print("\n[5/7] GPU availability")
    try:
        import torch
        if torch.cuda.is_available():
            n = torch.cuda.device_count()
            ok(f"CUDA available — {n} GPU(s)")
            for i in range(n):
                name = torch.cuda.get_device_name(i)
                mem = torch.cuda.get_device_properties(i).total_memory / (1024**3)
                ok(f"  GPU {i}: {name} ({mem:.0f} GB)")
            if n < 8:
                warn(f"Only {n} GPU(s) detected — pipeline expects 8× GPUs")
        else:
            warn("CUDA not available — training requires GPUs")
    except ImportError:
        warn("torch not installed — cannot check GPU")


# ═══════════════════════════════════════════════════════════════════
# 5. Eval data integrity
# ═══════════════════════════════════════════════════════════════════

def check_eval_data(work: str, strict: bool):
    print("\n[6/7] Evaluation data integrity")
    eval_dir = os.path.join(work, "training_data", "evaluation")
    if not os.path.isdir(eval_dir):
        fail(f"Eval directory missing: {eval_dir}", strict)
        return

    total_examples = 0
    for fname in sorted(os.listdir(eval_dir)):
        if not fname.endswith(".jsonl"):
            continue
        fpath = os.path.join(eval_dir, fname)
        line_count = 0
        parse_errors = 0
        with open(fpath, "r", encoding="utf-8") as f:
            for line_num, line in enumerate(f, 1):
                line = line.strip()
                if not line:
                    continue
                try:
                    obj = json.loads(line)
                    line_count += 1
                    # Basic schema checks
                    if "id" not in obj and "test_name" not in obj:
                        parse_errors += 1
                except json.JSONDecodeError:
                    parse_errors += 1

        total_examples += line_count
        if parse_errors > 0:
            fail(f"{fname}: {parse_errors} parse errors in {line_count} lines", strict)
        else:
            ok(f"{fname}: {line_count} valid examples")

    if total_examples >= 500:
        ok(f"Total eval examples: {total_examples} (≥500 threshold met)")
    else:
        warn(f"Total eval examples: {total_examples} (target ≥500)")


# ═══════════════════════════════════════════════════════════════════
# 6. Script executability
# ═══════════════════════════════════════════════════════════════════

def check_scripts(work: str, strict: bool):
    print("\n[7/7] Script permissions & syntax")
    scripts = [
        "training/run_all.sh",
        "training/setup_vast.sh",
    ]
    for rel in scripts:
        p = os.path.join(work, rel)
        if not os.path.isfile(p):
            continue
        if os.access(p, os.X_OK):
            ok(f"{rel} is executable")
        else:
            warn(f"{rel} is not executable (run: chmod +x {rel})")

    # Check Python scripts parse without syntax errors
    py_scripts = [
        "training/train_phase1_dapt.py",
        "training/train_phase2_sft.py",
        "training/eval_harness.py",
        "training/prepare_data.py",
        "training/export_model.py",
        "training_data/evaluation/generate_eval_data.py",
    ]
    for rel in py_scripts:
        p = os.path.join(work, rel)
        if not os.path.isfile(p):
            continue
        try:
            with open(p, "r") as f:
                compile(f.read(), p, "exec")
            ok(f"{rel} — syntax OK")
        except SyntaxError as e:
            fail(f"{rel} — syntax error: {e}", strict)


# ═══════════════════════════════════════════════════════════════════

def main():
    parser = argparse.ArgumentParser(description="Validate ApexIntel training pipeline")
    parser.add_argument("--work-dir", default=".", help="Workspace root")
    parser.add_argument("--strict", action="store_true", help="Treat warnings as errors")
    args = parser.parse_args()
    work = os.path.abspath(args.work_dir)

    print("═" * 60)
    print("  ApexIntel Training Pipeline — Pre-flight Validation")
    print("═" * 60)
    print(f"  Workspace: {work}")

    check_files(work, args.strict)
    check_configs(work, args.strict)
    check_dependencies(args.strict)
    check_gpu()
    check_eval_data(work, args.strict)
    check_scripts(work, args.strict)

    print("\n" + "═" * 60)
    if errors:
        print(f"  {FAIL} {len(errors)} ERROR(s), {len(warnings)} warning(s)")
        for e in errors:
            print(f"     {FAIL} {e}")
        print("═" * 60)
        sys.exit(1)
    elif warnings:
        print(f"  {PASS} No errors, {len(warnings)} warning(s)")
        for w in warnings:
            print(f"     {WARN} {w}")
        print("═" * 60)
        sys.exit(0)
    else:
        print(f"  {PASS} All checks passed — pipeline is ready!")
        print("═" * 60)
        sys.exit(0)


if __name__ == "__main__":
    main()
