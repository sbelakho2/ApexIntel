#!/usr/bin/env python3
"""Pre-quantize the model to 4-bit NF4, spread across 8 GPUs for the initial
conversion, then save. The saved model can be loaded on a single GPU for QLoRA."""
import os
import sys
from pathlib import Path

try:
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig
except ImportError as e:
    print(f"ERROR: Required package not installed: {e}", flush=True)
    sys.exit(1)

WORK = Path(__file__).resolve().parent.parent
MODEL_PATH = os.environ.get("MODEL_DIR", str(WORK / "models" / "base"))
OUT_PATH = os.environ.get("QUANTIZED_DIR", str(WORK / "models" / "base_4bit"))

print(f"Quantizing {MODEL_PATH} → {OUT_PATH}", flush=True)

# Check that input model exists
if not os.path.isdir(MODEL_PATH):
    print(f"FATAL: Model directory not found at {MODEL_PATH}", flush=True)
    print("Run training/download_model.py first, or set MODEL_DIR env var", flush=True)
    sys.exit(1)

# Check GPU availability
try:
    gpu_count = torch.cuda.device_count()
    print(f"GPUs: {gpu_count}", flush=True)
    if gpu_count == 0:
        print("FATAL: No CUDA GPUs available", flush=True)
        sys.exit(1)
except Exception as e:
    print(f"FATAL: Could not detect GPUs: {e}", flush=True)
    sys.exit(1)

try:
    # Check system memory availability (rough check)
    import psutil
    mem = psutil.virtual_memory()
    min_ram_gb = 32  # 30B model needs ~30GB + overhead on CPU for loading
    if mem.available / (1024 ** 3) < min_ram_gb:
        print(f"WARNING: Low available RAM ({mem.available / 1e9:.0f} GB). "
              f"Model loading may fail; consider --low-cpu-mem option if available.", flush=True)
except ImportError:
    print("  psutil not available, skipping memory check", flush=True)

bnb_config = BitsAndBytesConfig(
    load_in_4bit=True,
    bnb_4bit_quant_type="nf4",
    bnb_4bit_compute_dtype=torch.bfloat16,
    bnb_4bit_use_double_quant=True,
)

# Load with device_map="auto" to spread bf16 weights across ALL GPUs during quantization
try:
    model = AutoModelForCausalLM.from_pretrained(
        MODEL_PATH,
        quantization_config=bnb_config,
        device_map="auto",
        trust_remote_code=True,
        torch_dtype=torch.bfloat16,
    )
except torch.cuda.OutOfMemoryError as e:
    print(f"FATAL: CUDA OOM during model loading: {e}", flush=True)
    print("  Try reducing batch size or offloading more layers to CPU", flush=True)
    sys.exit(1)
except (OSError, IOError) as e:
    print(f"FATAL: Error reading model files at {MODEL_PATH}: {e}", flush=True)
    sys.exit(1)
except Exception as e:
    print(f"FATAL: Unexpected error loading model: {e}", flush=True)
    sys.exit(1)

print(f"Model loaded and quantized. Saving to {OUT_PATH}...", flush=True)

try:
    os.makedirs(OUT_PATH, exist_ok=True)
    model.save_pretrained(OUT_PATH)
    print("  ✓ Model weights saved", flush=True)
except (OSError, IOError) as e:
    print(f"FATAL: Failed to save quantized model: {e}", flush=True)
    sys.exit(1)

try:
    tokenizer = AutoTokenizer.from_pretrained(MODEL_PATH, trust_remote_code=True)
    tokenizer.save_pretrained(OUT_PATH)
    print("  ✓ Tokenizer saved", flush=True)
except Exception as e:
    print(f"WARNING: Failed to save tokenizer: {e}", flush=True)

# Validate output
try:
    # Verify output model can be loaded (quick test)
    model_size = sum(
        os.path.getsize(os.path.join(dirpath, f))
        for dirpath, _, filenames in os.walk(OUT_PATH)
        for f in filenames if f.endswith((".safetensors", ".bin"))
    )
    print(f"  ✓ Quantized model size: {model_size / 1e9:.1f} GB", flush=True)
    print("Done! 4-bit model saved.", flush=True)
except Exception as e:
    print(f"WARNING: Output validation failed: {e}", flush=True)
    print("Model may be incomplete.", flush=True)
    sys.exit(1)
