#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────
# ApexIntel — vast.ai 2×B200 GPU bootstrap
# Run this ONCE when the instance first starts.
#   Usage:  bash setup_vast.sh [HF_TOKEN]
# ──────────────────────────────────────────────────────────────────────
set -euo pipefail

HF_TOKEN="${1:-${HF_TOKEN:-}}"
WORK="${WORK_DIR:-/workspace/ApexIntel}"
MODEL_ID="${MODEL_ID:-Qwen/Qwen3-30B-A3B}"

echo "═══════════════════════════════════════════════"
echo "  ApexIntel Training Setup — 2×B200 (366 GB)"
echo "═══════════════════════════════════════════════"

# ── 0. Disk space check ────────────────────────────────────────────
echo "[0/8] Disk space check …"
AVAIL_GB=$(df -BG /workspace 2>/dev/null | tail -1 | awk '{print $4}' | tr -d 'G' || echo "0")
echo "  Available: ${AVAIL_GB}GB"
if [ "$AVAIL_GB" -lt 180 ]; then
    echo "  ⚠  WARNING: Less than 180GB free. Model download may fail."
    echo "  Cleaning pip/HF cache first …"
    pip cache purge 2>/dev/null || true
    rm -rf /root/.cache/pip 2>/dev/null || true
fi

# ── 1. System packages ─────────────────────────────────────────────
echo "[1/8] System packages …"
apt-get update -qq && apt-get install -yqq git-lfs tmux htop nvtop jq pigz 2>/dev/null || true
git lfs install --skip-smudge 2>/dev/null || true

# ── 2. Python environment ──────────────────────────────────────────
echo "[2/8] Python environment …"
pip install --upgrade pip setuptools wheel 2>/dev/null
pip install --no-cache-dir -r "$WORK/training/requirements.txt" 2>&1 | tail -5

# Flash Attention 2 (B200 / Blackwell)
pip install --no-cache-dir flash-attn --no-build-isolation 2>&1 | tail -3 || echo "WARN: flash-attn build failed, will fall back to sdpa"

# ── 3. HuggingFace auth ────────────────────────────────────────────
echo "[3/8] HuggingFace auth …"
if [ -n "$HF_TOKEN" ]; then
    huggingface-cli login --token "$HF_TOKEN" --add-to-git-credential
    echo "  ✓ authenticated"
else
    echo "  ⚠  No HF_TOKEN — set HF_TOKEN env or pass as argument"
fi

# ── 4. Download base model (disk-efficient) ────────────────────────
echo "[4/8] Downloading base model: $MODEL_ID …"
MODEL_DIR="$WORK/models/base"
mkdir -p "$MODEL_DIR"
if [ "$(find "$MODEL_DIR" -name '*.safetensors' 2>/dev/null | wc -l)" -gt 0 ]; then
    echo "  ✓ model already downloaded"
else
    # Use --local-dir to avoid HF cache duplication (saves ~160GB disk)
    HF_HUB_ENABLE_HF_TRANSFER=1 huggingface-cli download "$MODEL_ID" \
        --local-dir "$MODEL_DIR" \
        --exclude "*.gguf" "*.ot" "*.msgpack" "consolidated*" "original/*" \
        --resume-download \
        --local-dir-use-symlinks=False
    # Clean HF cache to reclaim disk
    rm -rf /root/.cache/huggingface/hub 2>/dev/null || true
    echo "  ✓ model downloaded to $MODEL_DIR"
fi

# ── 5. Verify GPU topology ─────────────────────────────────────────
echo "[5/8] GPU topology …"
python3 -c "
import os
import torch
n = torch.cuda.device_count()
print(f'  GPUs detected: {n}')
total_vram = 0
for i in range(n):
    name = torch.cuda.get_device_name(i)
    mem = torch.cuda.get_device_properties(i).total_memory / (1024**3)
    total_vram += mem
    print(f'    GPU {i}: {name}  ({mem:.0f} GB)')
print(f'  Total VRAM: {total_vram:.0f} GB')
# NOTE: This instance is 2×B200 (366 GB). The GPU count assertion below
# is set to match the actual hardware. If deploying on a different instance
# type, update the expected GPU count or pass via env var.
expected_gpus = int(os.environ.get("EXPECTED_GPUS", "2"))
assert n >= expected_gpus, f'Need {expected_gpus} GPUs, found {n}'
print(f'  ✓ {expected_gpus} GPUs available')
"

# ── 6. NCCL topology check ─────────────────────────────────────────
echo "[6/8] NCCL topology …"
python3 -c "
import torch
import torch.distributed as dist
# Quick check: can we see NVLink?
if hasattr(torch.cuda, 'can_device_access_peer'):
    for i in range(min(torch.cuda.device_count(), 8)):
        for j in range(i+1, min(torch.cuda.device_count(), 8)):
            p2p = torch.cuda.can_device_access_peer(i, j)
            if not p2p:
                print(f'  ⚠  No P2P between GPU {i} and GPU {j}')
    print('  ✓ P2P topology checked')
else:
    print('  ⚠  Cannot check P2P (torch too old)')
" 2>/dev/null || echo "  ⚠  NCCL topology check skipped"

# ── 7. Prepare training data ───────────────────────────────────────
echo "[7/8] Preparing training data …"
python3 "$WORK/training/prepare_data.py" --work-dir "$WORK"

# ── 8. Validate setup ──────────────────────────────────────────────
echo "[8/8] Validation …"
python3 -c "
import torch, transformers, peft, trl, datasets, accelerate, deepspeed
print(f'  torch        {torch.__version__}  (CUDA {torch.version.cuda})')
print(f'  transformers {transformers.__version__}')
print(f'  peft         {peft.__version__}')
print(f'  trl          {trl.__version__}')
print(f'  datasets     {datasets.__version__}')
print(f'  accelerate   {accelerate.__version__}')
print(f'  deepspeed    {deepspeed.__version__}')
try:
    import flash_attn
    print(f'  flash_attn   {flash_attn.__version__}')
except ImportError:
    print('  flash_attn   NOT INSTALLED (will use sdpa)')
"

# Disk usage summary
echo ""
echo "  Disk usage:"
du -sh "$WORK/models/base" 2>/dev/null | awk '{print "    Model: " $1}'
du -sh "$WORK/training/data" 2>/dev/null | awk '{print "    Training data: " $1}'
df -h /workspace 2>/dev/null | tail -1 | awk '{print "    Disk: " $3 " used / " $2 " total (" $5 " used)"}'

echo ""
echo "═══════════════════════════════════════════════"
echo "  Setup complete. Run training with:"
echo "    cd $WORK && bash training/run_all.sh"
echo "═══════════════════════════════════════════════"
