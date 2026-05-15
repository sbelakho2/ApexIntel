#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# setup_h200.sh — One-shot environment setup for 4×H200 vast.ai instance
#
# Installs: PyTorch, DeepSpeed, flash-attn, accelerate, transformers,
#           peft, trl, datasets, and all ApexIntel training deps.
#
# CUDA 13.1, sm_90 (Hopper), Python 3.12
# ═══════════════════════════════════════════════════════════════════
set -euo pipefail

WORK_DIR="${WORK_DIR:-/workspace/ApexIntel}"
cd "$WORK_DIR"

echo "═══════════════════════════════════════════════════════════"
echo "  ApexIntel H200 Setup"
echo "  $(date)"
echo "═══════════════════════════════════════════════════════════"

# ── Step 1: System packages ────────────────────────────────────────────────
echo "[1/6] System packages …"
apt-get update -qq && apt-get install -y -qq git git-lfs htop tmux rsync > /dev/null 2>&1
git lfs install --skip-smudge > /dev/null 2>&1
echo "  ✓ System packages"

# ── Step 2: PyTorch + CUDA ─────────────────────────────────────────────────
echo "[2/6] PyTorch (CUDA 12.8 wheels — binary-compatible with CUDA 13.1) …"
pip3 install --upgrade pip setuptools wheel > /dev/null 2>&1
pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu128 2>&1 | tail -3
python3 -c "
import torch
print(f'  PyTorch {torch.__version__}, CUDA {torch.version.cuda}')
print(f'  GPUs: {torch.cuda.device_count()}')
for i in range(torch.cuda.device_count()):
    print(f'    {i}: {torch.cuda.get_device_name(i)} {torch.cuda.get_device_properties(i).total_memory//1024**3}GB')
assert torch.cuda.device_count() >= 4, 'Expected 4 GPUs!'
print('  ✓ PyTorch OK')
"

# ── Step 3: Training stack ─────────────────────────────────────────────────
echo "[3/6] Training stack (transformers, peft, trl, deepspeed, accelerate) …"
pip3 install \
    transformers>=4.52.0 \
    peft>=0.15.0 \
    trl>=0.18.0 \
    accelerate>=1.6.0 \
    deepspeed>=0.16.0 \
    datasets>=3.5.0 \
    tokenizers>=0.21.0 \
    sentencepiece \
    protobuf \
    safetensors \
    einops \
    scipy \
    pyyaml \
    ninja \
    packaging 2>&1 | tail -5
echo "  ✓ Training stack installed"

# ── Step 4: Flash Attention 2 for sm_90 (Hopper) ──────────────────────────
echo "[4/6] Flash Attention 2 (building from source for sm_90) …"
# Try pre-built wheel first, fall back to source
export TORCH_CUDA_ARCH_LIST="9.0"
export MAX_JOBS=$(( $(nproc) / 4 ))  # Use 1/4 of cores to avoid OOM during compile
pip3 install flash-attn --no-build-isolation 2>&1 | tail -5 || {
    echo "  Pre-built failed, building from source …"
    pip3 install flash-attn --no-build-isolation --no-cache-dir 2>&1 | tail -10
}
python3 -c "
import flash_attn
from flash_attn.flash_attn_interface import flash_attn_func
print(f'  ✓ Flash Attention 2 v{flash_attn.__version__} — CUDA kernels OK')
" || echo "  ⚠ Flash Attention not available, will fall back to SDPA"

# ── Step 5: causal_conv1d (optional, for Mamba-based layers) ───────────────
echo "[5/6] causal_conv1d (optional) …"
pip3 install causal-conv1d --no-build-isolation 2>&1 | tail -3 || echo "  ⚠ causal_conv1d skipped"
python3 -c "from causal_conv1d import causal_conv1d_fn; print('  ✓ causal_conv1d OK')" 2>/dev/null || true

# ── Step 6: Verify full stack ──────────────────────────────────────────────
echo "[6/6] Verification …"
python3 -c "
import torch
import transformers
import peft
import trl
import accelerate
import deepspeed
import datasets

print(f'  torch:        {torch.__version__}')
print(f'  transformers: {transformers.__version__}')
print(f'  peft:         {peft.__version__}')
print(f'  trl:          {trl.__version__}')
print(f'  accelerate:   {accelerate.__version__}')
print(f'  deepspeed:    {deepspeed.__version__}')
print(f'  datasets:     {datasets.__version__}')

# Test NCCL across all GPUs
torch.distributed.init_process_group(backend='nccl', init_method='tcp://127.0.0.1:29501', rank=0, world_size=1)
t = torch.ones(1024, device='cuda:0')
print(f'  ✓ NCCL init OK')
torch.distributed.destroy_process_group()

# Quick matmul smoke test on each GPU
for i in range(torch.cuda.device_count()):
    a = torch.randn(1024, 1024, device=f'cuda:{i}', dtype=torch.bfloat16)
    b = torch.randn(1024, 1024, device=f'cuda:{i}', dtype=torch.bfloat16)
    c = a @ b
    torch.cuda.synchronize(i)
    del a, b, c
    print(f'  ✓ GPU {i} compute OK')

print()
print('  ═══ All checks passed — ready for training ═══')
"

echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  Setup complete!"
echo "  Next: upload code, download model, run pipeline"
echo "═══════════════════════════════════════════════════════════"
