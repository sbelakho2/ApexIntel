#!/usr/bin/env python3
"""
train_phase1_dapt.py — Phase 1: Domain-Adaptive Pre-Training (QLoRA)
Causal LM on EMS/manufacturing domain corpus on 8×RTX 5090 (32 GB each).

Model: Qwen3-30B-A3B (30B total / 3B active MoE) | QLoRA 4-bit NF4

Uses 4-bit quantization so each GPU holds the full model (~15 GB in NF4).
Only LoRA gradients (106 MB) are synced via DDP — no ZeRO-3 all-gather overhead.

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
    BitsAndBytesConfig,
    TrainingArguments,
    Trainer,
    DataCollatorForLanguageModeling,
    set_seed,
)
from peft import LoraConfig, get_peft_model, TaskType, prepare_model_for_kbit_training

WORK = Path(__file__).resolve().parent.parent
CONFIG_PATH = WORK / "training" / "configs" / "phase1_dapt.yaml"


# ── CUDA Performance Tuning ──────────────────────────────────
def setup_cuda_optimizations():
    """Apply all CUDA-level performance optimizations for RTX 5090 Blackwell."""
    torch.backends.cuda.matmul.allow_tf32 = True
    torch.backends.cudnn.allow_tf32 = True
    torch.backends.cudnn.benchmark = True
    torch.backends.cudnn.deterministic = False
    torch.set_float32_matmul_precision("high")
    os.environ.setdefault("CUDA_LAUNCH_BLOCKING", "0")
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

    local_rank = int(os.environ.get("LOCAL_RANK", 0))

    print(f"Phase 1 DAPT (QLoRA 4-bit) — model: {model_path}")
    print(f"  output: {output_dir}")
    print(f"  GPUs: {torch.cuda.device_count()}, local_rank: {local_rank}")

    # ── Tokenizer ──────────────────────────────────────────────────
    tokenizer = AutoTokenizer.from_pretrained(
        model_path,
        trust_remote_code=True,
        padding_side="right",
    )
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    # ── 4-bit Quantization Config ──────────────────────────────────
    # Pre-quantized NF4 model (~15 GB) — each GPU holds the full model
    bnb_config = BitsAndBytesConfig(
        load_in_4bit=True,
        bnb_4bit_quant_type="nf4",
        bnb_4bit_compute_dtype=torch.bfloat16,
        bnb_4bit_use_double_quant=True,
    )
    print("  ✓ 4-bit NF4 quantization config (double-quant enabled)")

    # ── Model ──────────────────────────────────────────────────────
    attn = cfg["model"].get("attn_implementation", "flash_attention_2")
    if attn == "flash_attention_2":
        import flash_attn  # noqa: F401
        from flash_attn.flash_attn_interface import flash_attn_func  # noqa: F401
        print(f"  ✓ Flash Attention 2 v{flash_attn.__version__} with CUDA kernels")

    # Try pre-quantized model first, fall back to base + quantize
    quant_path = str(WORK / "models" / "base_4bit")
    if os.path.exists(os.path.join(quant_path, "config.json")):
        print(f"  Loading pre-quantized 4-bit model from {quant_path}")
        model = AutoModelForCausalLM.from_pretrained(
            quant_path,
            quantization_config=bnb_config,
            attn_implementation=attn,
            trust_remote_code=True,
            use_cache=False,
            device_map={"": local_rank},
        )
    else:
        print(f"  No pre-quantized model found. Quantizing with device_map=auto...")
        model = AutoModelForCausalLM.from_pretrained(
            model_path,
            quantization_config=bnb_config,
            attn_implementation=attn,
            trust_remote_code=True,
            use_cache=False,
            device_map="auto",  # spread across GPUs for initial quantization
        )
    model.config.use_cache = False

    # Prepare for k-bit training (freeze base, fp32 norms, enable input grads)
    model = prepare_model_for_kbit_training(
        model,
        use_gradient_checkpointing=True,
        gradient_checkpointing_kwargs={"use_reentrant": False},
    )
    print("  ✓ Model prepared for QLoRA k-bit training")

    # ── LoRA ───────────────────────────────────────────────────────
    peft_config = LoraConfig(
        r=lora_cfg["r"],
        lora_alpha=lora_cfg["lora_alpha"],
        lora_dropout=lora_cfg["lora_dropout"],
        target_modules=lora_cfg["target_modules"],
        task_type=TaskType.CAUSAL_LM,
        bias=lora_cfg.get("bias", "none"),
        use_rslora=True,
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
            import time
            t0 = time.time()
            all_input_ids = []
            for i in range(0, len(dataset), batch_size):
                batch_texts = dataset[i : i + batch_size]["text"]
                encoded = tokenizer(
                    batch_texts, truncation=False,
                    add_special_tokens=False, return_attention_mask=False,
                )
                for ids in encoded["input_ids"]:
                    all_input_ids.extend(ids)
                    all_input_ids.append(tokenizer.eos_token_id)
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
        collator = DataCollatorForLanguageModeling(tokenizer=tokenizer, mlm=False)
        print(f"  Packed: {len(train_dataset)} train, {len(eval_dataset)} eval sequences")
    else:
        def tokenize_fn(examples):
            out = tokenizer(
                examples["text"], truncation=True, max_length=max_len, padding=False,
            )
            out["labels"] = out["input_ids"].copy()
            return out
        tokenized = raw.map(tokenize_fn, batched=True, remove_columns=["text"],
                            num_proc=4, desc="Tokenizing")
        train_dataset = tokenized["train"]
        eval_dataset = tokenized["eval"]
        collator = DataCollatorForLanguageModeling(tokenizer=tokenizer, mlm=False)

    # ── Training args — No DeepSpeed, pure DDP ─────────────────────
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
        eval_strategy="no",  # skip eval during short 1-epoch run
        dataloader_num_workers=tc["dataloader_num_workers"],
        dataloader_pin_memory=tc["dataloader_pin_memory"],
        dataloader_prefetch_factor=tc.get("dataloader_prefetch_factor", 4),
        dataloader_persistent_workers=tc.get("dataloader_persistent_workers", True),
        report_to=tc.get("report_to", "none"),
        run_name=tc.get("run_name", "apexintel-dapt"),
        gradient_checkpointing=True,
        gradient_checkpointing_kwargs={"use_reentrant": False},
        ddp_find_unused_parameters=False,
        torch_compile=False,
        seed=42,
        # No deepspeed — pure DDP, only LoRA grads synced (~106 MB)
    )

    # ── Trainer ────────────────────────────────────────────────────
    trainer = Trainer(
        model=model,
        args=training_args,
        train_dataset=train_dataset,
        eval_dataset=eval_dataset,
        data_collator=collator,
    )

    print("Starting Phase 1 DAPT training (QLoRA 4-bit) …")
    trainer.train()

    # ── Save adapter ───────────────────────────────────────────────
    best_dir = os.path.join(output_dir, "best_adapter")
    model.save_pretrained(best_dir)
    tokenizer.save_pretrained(best_dir)
    print(f"Phase 1 DAPT complete. Adapter saved to {best_dir}")


if __name__ == "__main__":
    main()
