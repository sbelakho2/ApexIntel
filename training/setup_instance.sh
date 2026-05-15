#!/bin/bash
set -euo pipefail
echo "======================================"
echo " ApexIntel SFT v3 — Instance Setup"
echo " 4×H200 FSDP Training Pipeline"
echo "======================================"

export DEBIAN_FRONTEND=noninteractive
export LC_ALL=C

# Use env vars with sensible defaults for portability
WORK_DIR="${WORK_DIR:-/workspace/ApexIntel}"
MODEL_DIR="${MODEL_DIR:-${WORK_DIR}/models/base}"
EXPECTED_GPUS="${EXPECTED_GPUS:-4}"

# ── 1. System packages ──────────────────────────
echo "[1/6] Installing system packages..."
apt-get update -qq
apt-get install -y -qq git curl wget rsync htop nvtop tmux > /dev/null 2>&1
echo "  ✓ System packages installed"

# ── 2. Python packages ──────────────────────────
echo "[2/6] Installing Python ML stack..."
pip install --quiet --upgrade pip
pip install --quiet \
    torch torchvision torchaudio \
    transformers accelerate peft trl \
    datasets tokenizers sentencepiece \
    bitsandbytes scipy scikit-learn \
    pyyaml safetensors huggingface_hub \
    flash-attn --no-build-isolation 2>&1 | tail -5
echo "  ✓ Python packages installed"

# ── 3. Verify GPU access ────────────────────────
echo "[3/6] Verifying GPU access..."
python3 -c "
import torch, os
n = torch.cuda.device_count()
expected = int(os.environ.get('EXPECTED_GPUS', '${EXPECTED_GPUS}'))
print(f'  ✓ PyTorch {torch.__version__}, CUDA {torch.version.cuda}')
for i in range(n):
    name = torch.cuda.get_device_name(i)
    mem = torch.cuda.get_device_properties(i).total_mem / 1e9
    print(f'  GPU {i}: {name} ({mem:.0f} GB)')
assert n >= expected, f'Need at least {expected} GPUs, found {n}'
print(f'  ✓ {n} GPUs available')
"

# ── 4. Download base model ──────────────────────
echo "[4/6] Downloading Qwen3-30B-A3B base model..."
if [ -f "$MODEL_DIR/config.json" ]; then
    echo "  ✓ Model already exists at $MODEL_DIR"
else
    python3 -c "
from huggingface_hub import snapshot_download
import os
token = os.environ.get('HF_TOKEN', '')
model_id = os.environ.get('MODEL_ID', 'Qwen/Qwen3-30B-A3B')
print(f'  Downloading {model_id} ...')
snapshot_download(
    model_id,
    local_dir='$MODEL_DIR',
    token=token if token else None,
    ignore_patterns=['*.gguf', '*.ggml'],
)
print('  ✓ Model downloaded')
"
fi

# ── 5. Verify data files ────────────────────────
echo "[5/6] Verifying data files..."
for f in "${WORK_DIR}/training/data/sft_train.jsonl" "${WORK_DIR}/training/data/sft_eval.jsonl"; do
    if [ -f "$f" ]; then
        count=$(wc -l < "$f")
        size=$(du -sh "$f" | cut -f1)
        echo "  ✓ $(basename $f): $count lines ($size)"
    else
        echo "  ✗ MISSING: $f"
        exit 1
    fi
done
echo "  Eval files:"
ls -1 "${WORK_DIR}/training_data/evaluation/"*.jsonl 2>/dev/null | while read f; do
    echo "    $(basename $f): $(wc -l < "$f") examples"
done

# ── 6. Verify configs ───────────────────────────
echo "[6/6] Verifying configs..."
for f in "${WORK_DIR}/training/configs/phase2_sft.yaml" "${WORK_DIR}/training/configs/accelerate_fsdp_${EXPECTED_GPUS}gpu.yaml"; do
    if [ -f "$f" ]; then
        echo "  ✓ $(basename $f)"
    else
        echo "  ✗ MISSING: $f"
        exit 1
    fi
done

echo ""
echo "======================================"
echo " Setup complete! Ready to train."
echo "======================================"
echo ""
echo "Model: $MODEL_DIR"
echo "GPU config: $(nvidia-smi --query-gpu=name --format=csv,noheader | head -1) × $(nvidia-smi --query-gpu=name --format=csv,noheader | wc -l)"
echo "Disk: $(df -h / | tail -1 | awk '{print $3 "/" $2 " (" $5 " used)"}')"
