#!/usr/bin/env python3
"""
pre_merge_phase1.py — Merge Phase 1 DAPT adapter into base model.

Runs single-process on CPU before the distributed SFT launch.
Saves the merged model to training/outputs/merged_phase1/ so that
the ZeRO-3 training script can load it with proper weight partitioning.

Usage (called automatically by run_all.sh):
    python3 training/pre_merge_phase1.py
"""

import os
import sys
import time
import yaml
import torch
from pathlib import Path
from transformers import AutoModelForCausalLM, AutoTokenizer
from peft import PeftModel

WORK = Path(__file__).resolve().parent.parent
CONFIG_PATH = WORK / "training" / "configs" / "phase2_sft.yaml"
MERGED_DIR = WORK / "training" / "outputs" / "merged_phase1"


def main():
    with open(CONFIG_PATH) as f:
        cfg = yaml.safe_load(f)

    model_path = str(WORK / cfg["model"]["name_or_path"])
    phase1_adapter = str(WORK / cfg["model"]["phase1_adapter"])

    # Skip if already merged
    if (MERGED_DIR / "config.json").exists():
        print(f"[pre-merge] Merged model already exists at {MERGED_DIR}, skipping.")
        return

    if not os.path.isdir(phase1_adapter):
        print(f"[pre-merge] Phase 1 adapter not found at {phase1_adapter}, nothing to merge.")
        # Create a sentinel so training script loads from base
        MERGED_DIR.mkdir(parents=True, exist_ok=True)
        (MERGED_DIR / ".no_adapter").touch()
        return

    print(f"[pre-merge] Loading base model from {model_path} ...")
    t0 = time.time()

    model = AutoModelForCausalLM.from_pretrained(
        model_path,
        dtype=torch.bfloat16,
        trust_remote_code=True,
        low_cpu_mem_usage=True,
        device_map="cpu",
    )
    print(f"[pre-merge] Base model loaded in {time.time() - t0:.1f}s")

    print(f"[pre-merge] Loading Phase 1 adapter from {phase1_adapter} ...")
    t1 = time.time()
    model = PeftModel.from_pretrained(model, phase1_adapter)
    print(f"[pre-merge] Adapter loaded in {time.time() - t1:.1f}s")

    print("[pre-merge] Merging adapter into base model ...")
    t2 = time.time()
    model = model.merge_and_unload()
    print(f"[pre-merge] Merge complete in {time.time() - t2:.1f}s")

    print(f"[pre-merge] Saving merged model to {MERGED_DIR} ...")
    t3 = time.time()
    MERGED_DIR.mkdir(parents=True, exist_ok=True)
    model.save_pretrained(MERGED_DIR, safe_serialization=True)

    # Also save tokenizer alongside model for convenience
    tokenizer = AutoTokenizer.from_pretrained(model_path, trust_remote_code=True)
    tokenizer.save_pretrained(MERGED_DIR)

    print(f"[pre-merge] Saved in {time.time() - t3:.1f}s")
    print(f"[pre-merge] Total time: {time.time() - t0:.1f}s")

    # Free memory
    del model
    import gc
    gc.collect()


if __name__ == "__main__":
    main()
