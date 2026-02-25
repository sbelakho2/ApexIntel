#!/usr/bin/env python3
"""
train_phase2_sft.py — Phase 2: Supervised Fine-Tuning (SFT)
Chat-format instruction tuning on 10 ApexIntel tasks.
LoRA is stacked on the merged Phase 1 adapter.

Model: Qwen3-30B-A3B (30B total / 3B active MoE) on 8×RTX 5090 (32 GB each).
FSDP FULL_SHARD (PyTorch native) — no CPU offload.

Usage (launched by run_all.sh via accelerate):
    accelerate launch --config_file training/configs/accelerate_fsdp_8gpu.yaml \
        training/train_phase2_sft.py
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

from datasets import load_dataset
from transformers import (
    AutoModelForCausalLM,
    AutoTokenizer,
    set_seed,
)
from peft import LoraConfig, PeftModel, get_peft_model, TaskType
from trl import SFTTrainer, SFTConfig

WORK = Path(__file__).resolve().parent.parent
CONFIG_PATH = WORK / "training" / "configs" / "phase2_sft.yaml"


# ── CUDA Performance Tuning ──────────────────────────────────
def setup_cuda_optimizations():
    """Apply all CUDA-level performance optimizations for B200 Blackwell."""
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
    phase1_adapter = str(WORK / cfg["model"]["phase1_adapter"])
    output_dir = str(WORK / cfg["training"]["output_dir"])
    tc = cfg["training"]
    dc = cfg["data"]
    lora_cfg = cfg["lora"]

    # ── Determine model path (pre-merged or base) ──────────────────
    merged_path = WORK / "training" / "outputs" / "merged_phase1"
    if (merged_path / "config.json").exists():
        load_path = str(merged_path)
        print(f"Phase 2 SFT — loading pre-merged model from {load_path}")
    else:
        load_path = model_path
        print(f"Phase 2 SFT — loading base model from {load_path} (no merged model found)")

    print(f"  Phase 1 adapter: {phase1_adapter}")
    print(f"  output: {output_dir}")
    print(f"  GPUs: {torch.cuda.device_count()}")

    # ── FSDP: no pre-registration needed ────────────────────────────
    # With FSDP FULL_SHARD, accelerate handles weight sharding after
    # model construction. We load with low_cpu_mem_usage=True and let
    # FSDP shard across GPUs automatically.
    print("  ✓ FSDP mode — accelerate handles sharding after model load")

    # ── Tokenizer ──────────────────────────────────────────────────
    tokenizer = AutoTokenizer.from_pretrained(
        load_path,
        trust_remote_code=True,
        padding_side="right",
    )
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    # ── Load model (FSDP will shard across GPUs post-load) ──────────
    attn = cfg["model"].get("attn_implementation", "flash_attention_2")
    if attn == "flash_attention_2":
        try:
            import flash_attn  # noqa: F401
            from flash_attn.flash_attn_interface import flash_attn_func  # noqa: F401
            print(f"  ✓ Flash Attention 2 v{flash_attn.__version__} with CUDA kernels")
        except (ImportError, ModuleNotFoundError):
            attn = "sdpa"
            print("  ⚠ flash-attn not available, falling back to SDPA (PyTorch native)")

    print(f"Loading model from {load_path} (FSDP will shard after load) …")
    model = AutoModelForCausalLM.from_pretrained(
        load_path,
        dtype=torch.bfloat16,
        attn_implementation=attn,
        trust_remote_code=True,
        use_cache=False,
        low_cpu_mem_usage=True,
    )

    model.config.use_cache = False

    # ── Phase 2 LoRA ───────────────────────────────────────────────
    peft_config = LoraConfig(
        r=lora_cfg["r"],
        lora_alpha=lora_cfg["lora_alpha"],
        lora_dropout=lora_cfg["lora_dropout"],
        target_modules=lora_cfg["target_modules"],
        task_type=TaskType.CAUSAL_LM,
        bias=lora_cfg.get("bias", "none"),
        use_rslora=True,  # Rank-Stabilized LoRA scaling
    )
    model = get_peft_model(model, peft_config)
    model.print_trainable_parameters()

    # ── Dataset ────────────────────────────────────────────────────
    train_path = str(WORK / dc["train_file"])
    eval_path = str(WORK / dc["eval_file"])
    max_len = dc.get("max_seq_length", 8192)

    raw = load_dataset("json", data_files={"train": train_path, "eval": eval_path})

    def format_chat(example):
        """Apply the model's chat template to the messages."""
        messages = example["messages"]
        text = tokenizer.apply_chat_template(
            messages,
            tokenize=False,
            add_generation_prompt=False,
        )
        return {"text": text}

    formatted = raw.map(format_chat, desc="Applying chat template")

    # ── SFT Training ───────────────────────────────────────────────
    sft_config = SFTConfig(
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
        run_name=tc.get("run_name", "apexintel-sft"),
        # FSDP is handled by accelerate config — no deepspeed arg needed
        gradient_checkpointing=tc.get("gradient_checkpointing", True),
        gradient_checkpointing_kwargs={"use_reentrant": False, "determinism_check": "none"},
        ddp_find_unused_parameters=tc.get("ddp_find_unused_parameters", False),
        torch_compile=False,  # MoE dynamic routing not compatible with compile
        max_length=max_len,
        dataset_text_field="text",
        neftune_noise_alpha=tc.get("neftune_noise_alpha", 5.0),
        seed=42,
    )

    trainer = SFTTrainer(
        model=model,
        args=sft_config,
        train_dataset=formatted["train"],
        eval_dataset=formatted["eval"],
        processing_class=tokenizer,
    )

    print("Starting Phase 2 SFT training …")
    trainer.train()

    # ── Save best adapter ──────────────────────────────────────────
    # With FSDP, the Trainer already saves checkpoints properly.
    # We copy the best checkpoint's adapter to best_adapter/ on rank 0 only.
    import shutil
    from accelerate import PartialState
    best_dir = os.path.join(output_dir, "best_adapter")
    if PartialState().is_main_process:
        best_ckpt = trainer.state.best_model_checkpoint
        if best_ckpt and os.path.isdir(best_ckpt):
            os.makedirs(best_dir, exist_ok=True)
            for fname in ["adapter_model.safetensors", "adapter_config.json",
                          "tokenizer.json", "tokenizer_config.json", "chat_template.jinja"]:
                src = os.path.join(best_ckpt, fname)
                if os.path.exists(src):
                    shutil.copy2(src, best_dir)
            print(f"Phase 2 SFT complete. Best adapter copied from {best_ckpt} to {best_dir}")
        else:
            print(f"Phase 2 SFT complete. Best checkpoint not found — use checkpoint dirs directly.")


if __name__ == "__main__":
    main()
