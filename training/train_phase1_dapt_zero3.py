#!/usr/bin/env python3
"""
train_phase1_dapt.py — Phase 1: Domain-Adaptive Pre-Training (LoRA)
Causal LM on EMS/manufacturing domain corpus on 8×RTX 5090 (32 GB each).

Model: Qwen3-30B-A3B (30B total / 3B active MoE) | DeepSpeed ZeRO-3

Usage (launched by run_all.sh via accelerate):
    accelerate launch --config_file training/configs/accelerate_8gpu.yaml \
        training/train_phase1_dapt.py
"""

import os
import sys
import yaml

# PyTorch 2.10 introduced a strict metadata check in use_reentrant=False GC that
# fires on Qwen3-MoE due to flash-attn 2.8 producing different strides on
# recompute. The data is identical; only the memory layout differs. Patch the
# extractor to return None so the check always passes.
import torch.utils.checkpoint as _ckpt
_ckpt._default_meta_extractor = lambda x: None
import torch
from pathlib import Path

from datasets import load_dataset, Dataset
from transformers import (
    AutoModelForCausalLM,
    AutoTokenizer,
    TrainingArguments,
    Trainer,
    DataCollatorForLanguageModeling,
    set_seed,
)
from peft import LoraConfig, get_peft_model, TaskType

WORK = Path(__file__).resolve().parent.parent
CONFIG_PATH = WORK / "training" / "configs" / "phase1_dapt.yaml"


# ── CUDA Performance Tuning ──────────────────────────────────
def setup_cuda_optimizations():
    """Apply all CUDA-level performance optimizations for B200 Blackwell."""
    # TF32: allow TF32 for any internal fp32 matmuls (3x faster than fp32)
    torch.backends.cuda.matmul.allow_tf32 = True
    torch.backends.cudnn.allow_tf32 = True
    # cuDNN benchmark: auto-tune convolution algorithms
    torch.backends.cudnn.benchmark = True
    torch.backends.cudnn.deterministic = False
    # High-precision matmul mode (uses TF32 on Ampere+)
    torch.set_float32_matmul_precision("high")
    # Disable debug sync for max throughput
    os.environ.setdefault("CUDA_LAUNCH_BLOCKING", "0")
    # Note: expandable_segments removed — causes progressive fragmentation with ZeRO-3
    print("  ✓ CUDA optimizations: TF32, cuDNN benchmark")


def load_config() -> dict:
    with open(CONFIG_PATH) as f:
        return yaml.safe_load(f)


def main():
    cfg = load_config()
    set_seed(42)
    setup_cuda_optimizations()

    model_path = str(WORK / cfg["model"]["name_or_path"])
    output_dir = str(WORK / cfg["training"]["output_dir"])
    tc = cfg["training"]
    dc = cfg["data"]
    lora_cfg = cfg["lora"]

    # ── CRITICAL: Enable ZeRO-3 Init BEFORE model loading ─────────
    # Without this, from_pretrained loads the full 30B model (~57GB bf16)
    # on EACH process, causing OOM (32GB per GPU). HfDeepSpeedConfig
    # registers a global flag so from_pretrained uses deepspeed.zero.Init()
    # to shard parameters across all GPUs during loading.
    if tc.get("deepspeed"):
        from transformers.integrations import HfDeepSpeedConfig
        from transformers.integrations.deepspeed import is_deepspeed_zero3_enabled
        _dschf = HfDeepSpeedConfig(str(WORK / tc["deepspeed"]))  # must stay in scope
        assert is_deepspeed_zero3_enabled(), (
            "ZeRO-3 init failed to activate — cannot load 30B model without sharding"
        )
        print(f"  ✓ ZeRO-3 init context active (params sharded during load)")

    print(f"Phase 1 DAPT — model: {model_path}")
    print(f"  output: {output_dir}")
    print(f"  GPUs: {torch.cuda.device_count()}")

    # ── Tokenizer ──────────────────────────────────────────────────
    tokenizer = AutoTokenizer.from_pretrained(
        model_path,
        trust_remote_code=True,
        padding_side="right",
    )
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    # ── Model ──────────────────────────────────────────────────────
    # NOTE: We rely on accelerate's zero3_init_flag=true to handle ZeRO-3 sharding.
    # The accelerate config initializes distributed BEFORE model loading,
    # so ZeRO-3 can shard parameters correctly across all GPUs.
    attn = cfg["model"].get("attn_implementation", "flash_attention_2")
    # Require flash_attn — DO NOT fall back to sdpa
    if attn == "flash_attention_2":
        import flash_attn  # noqa: F401
        from flash_attn.flash_attn_interface import flash_attn_func  # verify CUDA kernels
        print(f"  ✓ Flash Attention 2 v{flash_attn.__version__} with CUDA kernels")
    model = AutoModelForCausalLM.from_pretrained(
        model_path,
        dtype=torch.bfloat16,
        attn_implementation=attn,
        trust_remote_code=True,
        use_cache=False,  # required for gradient checkpointing
        low_cpu_mem_usage=True,
    )
    model.config.use_cache = False

    # ── LoRA ───────────────────────────────────────────────────────
    peft_config = LoraConfig(
        r=lora_cfg["r"],
        lora_alpha=lora_cfg["lora_alpha"],
        lora_dropout=lora_cfg["lora_dropout"],
        target_modules=lora_cfg["target_modules"],
        task_type=TaskType.CAUSAL_LM,
        bias=lora_cfg.get("bias", "none"),
        use_rslora=True,  # Rank-Stabilized LoRA: scales alpha by 1/sqrt(r) for stable training
    )
    model = get_peft_model(model, peft_config)
    model.print_trainable_parameters()

    # ── Dataset ────────────────────────────────────────────────────
    train_path = str(WORK / dc["train_file"])
    eval_path = str(WORK / dc["eval_file"])
    max_len = dc.get("max_seq_length", 4096)
    packing = dc.get("packing", False)

    raw = load_dataset("json", data_files={"train": train_path, "eval": eval_path})

    if packing:
        print(f"  Packing enabled — concatenating+chunking to {max_len} tokens")

        def pack_dataset(dataset, max_length, batch_size=1000):
            """Batch-tokenize all texts, concatenate, chunk into fixed-length sequences."""
            import time
            t0 = time.time()
            all_input_ids = []

            # Batch tokenize for 10-50x speedup vs one-by-one
            for i in range(0, len(dataset), batch_size):
                batch_texts = dataset[i : i + batch_size]["text"]
                encoded = tokenizer(
                    batch_texts,
                    truncation=False,
                    add_special_tokens=False,
                    return_attention_mask=False,
                )
                for ids in encoded["input_ids"]:
                    all_input_ids.extend(ids)
                    all_input_ids.append(tokenizer.eos_token_id)

            # Chunk into fixed-length sequences
            n_chunks = (len(all_input_ids) - max_length + 1) // max_length
            chunks = [all_input_ids[i * max_length : (i + 1) * max_length] for i in range(n_chunks)]
            elapsed = time.time() - t0
            print(f"    → {n_chunks} sequences from {len(all_input_ids):,} tokens ({elapsed:.1f}s)")
            return Dataset.from_dict({
                "input_ids": chunks,
                "attention_mask": [[1] * max_length for _ in chunks],
                "labels": [c[:] for c in chunks],
            })

        train_dataset = pack_dataset(raw["train"], max_len)
        eval_dataset = pack_dataset(raw["eval"], max_len)
        collator = DataCollatorForLanguageModeling(
            tokenizer=tokenizer,
            mlm=False,
        )
        print(f"  Packed: {len(train_dataset)} train, {len(eval_dataset)} eval sequences")
    else:
        def tokenize_fn(examples):
            out = tokenizer(
                examples["text"],
                truncation=True,
                max_length=max_len,
                padding=False,
            )
            out["labels"] = out["input_ids"].copy()
            return out

        tokenized = raw.map(tokenize_fn, batched=True, remove_columns=["text"],
                            num_proc=4, desc="Tokenizing")
        train_dataset = tokenized["train"]
        eval_dataset = tokenized["eval"]
        collator = DataCollatorForLanguageModeling(
            tokenizer=tokenizer,
            mlm=False,
        )

    # ── Training args ──────────────────────────────────────────────
    training_args = TrainingArguments(
        output_dir=output_dir,
        num_train_epochs=tc["num_train_epochs"],
        per_device_train_batch_size=tc["per_device_train_batch_size"],
        per_device_eval_batch_size=tc["per_device_eval_batch_size"],
        gradient_accumulation_steps=tc["gradient_accumulation_steps"],
        learning_rate=tc["learning_rate"],
        weight_decay=tc["weight_decay"],
        warmup_ratio=tc["warmup_ratio"],
        lr_scheduler_type=tc["lr_scheduler_type"],
        max_grad_norm=tc["max_grad_norm"],
        bf16=tc["bf16"],
        logging_steps=tc["logging_steps"],
        save_strategy=tc["save_strategy"],
        save_steps=tc["save_steps"],
        save_total_limit=tc["save_total_limit"],
        eval_strategy=tc["eval_strategy"],
        eval_steps=tc["eval_steps"],
        load_best_model_at_end=tc["load_best_model_at_end"],
        metric_for_best_model=tc["metric_for_best_model"],
        greater_is_better=tc["greater_is_better"],
        dataloader_num_workers=tc["dataloader_num_workers"],
        dataloader_pin_memory=tc["dataloader_pin_memory"],
        dataloader_prefetch_factor=tc.get("dataloader_prefetch_factor", 4),
        dataloader_persistent_workers=tc.get("dataloader_persistent_workers", True),
        report_to=tc.get("report_to", "none"),
        run_name=tc.get("run_name", "apexintel-dapt"),
        deepspeed=str(WORK / tc["deepspeed"]) if tc.get("deepspeed") else None,
        gradient_checkpointing=tc.get("gradient_checkpointing", True),
        gradient_checkpointing_kwargs={"use_reentrant": False, "determinism_check": "none"},
        ddp_find_unused_parameters=tc.get("ddp_find_unused_parameters", False),
        torch_compile=False,  # MoE dynamic routing not compatible with compile
        seed=42,
    )

    # ── Trainer ────────────────────────────────────────────────────
    trainer = Trainer(
        model=model,
        args=training_args,
        train_dataset=train_dataset,
        eval_dataset=eval_dataset,
        data_collator=collator,
    )

    print("Starting Phase 1 DAPT training …")
    trainer.train()

    # ── Save best adapter ──────────────────────────────────────────
    best_dir = os.path.join(output_dir, "best_adapter")
    model.save_pretrained(best_dir)
    tokenizer.save_pretrained(best_dir)
    print(f"Phase 1 DAPT complete. Adapter saved to {best_dir}")


if __name__ == "__main__":
    main()
