#!/usr/bin/env python3
"""Pre-quantize the model to 4-bit NF4, spread across 8 GPUs for the initial
conversion, then save. The saved model can be loaded on a single GPU for QLoRA."""
import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig
from pathlib import Path

WORK = Path(__file__).resolve().parent.parent
MODEL_PATH = str(WORK / "models" / "base")
OUT_PATH = str(WORK / "models" / "base_4bit")

print(f"Quantizing {MODEL_PATH} → {OUT_PATH}")
print(f"GPUs: {torch.cuda.device_count()}")

bnb_config = BitsAndBytesConfig(
    load_in_4bit=True,
    bnb_4bit_quant_type="nf4",
    bnb_4bit_compute_dtype=torch.bfloat16,
    bnb_4bit_use_double_quant=True,
)

# Load with device_map="auto" to spread bf16 weights across ALL GPUs during quantization
model = AutoModelForCausalLM.from_pretrained(
    MODEL_PATH,
    quantization_config=bnb_config,
    device_map="auto",
    trust_remote_code=True,
    torch_dtype=torch.bfloat16,
)
print(f"Model loaded and quantized. Saving to {OUT_PATH}...")

model.save_pretrained(OUT_PATH)
tokenizer = AutoTokenizer.from_pretrained(MODEL_PATH, trust_remote_code=True)
tokenizer.save_pretrained(OUT_PATH)
print("Done! 4-bit model saved.")
