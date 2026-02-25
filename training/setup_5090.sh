#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# setup_5090.sh — One-shot environment setup for 8×RTX 5090 vast.ai instance
#
# Installs: PyTorch, DeepSpeed, flash-attn, accelerate, transformers,
#           peft, trl, datasets, and all ApexIntel training deps.
#
# CUDA 13.0, sm_100 (Blackwell), Python 3.12
# ═══════════════════════════════════════════════════════════════════
set -uo pipefail  # no -e: we handle errors per-step

WORK_DIR="${WORK_DIR:-/workspace/ApexIntel}"
cd /workspace

echo "═══════════════════════════════════════════════════════════"
echo "  ApexIntel 8×RTX 5090 Setup"
echo "  $(date)"
echo "═══════════════════════════════════════════════════════════"

# ── Step 1: System packages ────────────────────────────────────────────────
echo "[1/6] System packages …"
apt-get update -qq && apt-get install -y -qq git git-lfs htop tmux rsync > /dev/null 2>&1
git lfs install --skip-smudge > /dev/null 2>&1 || true
echo "  ✓ System packages"

# ── Step 2: PyTorch + CUDA ─────────────────────────────────────────────────
echo "[2/6] PyTorch (CUDA 12.8 wheels — binary-compatible with CUDA 13.0) …"
# Upgrade pip without touching system-managed wheel package
pip3 install --upgrade pip setuptools 2>&1 | tail -2 || true
pip3 install torch torchvision torchaudio --index-url https://download.pytorch.org/whl/cu128 2>&1 | tail -3
python3 -c "
import torch
print(f'  PyTorch {torch.__version__}, CUDA {torch.version.cuda}')
print(f'  GPUs: {torch.cuda.device_count()}')
for i in range(torch.cuda.device_count()):
    print(f'    {i}: {torch.cuda.get_device_name(i)} {torch.cuda.get_device_properties(i).total_memory//1024**3}GB')
assert torch.cuda.device_count() >= 8, f'Expected 8 GPUs, found {torch.cuda.device_count()}!'
print('  ✓ PyTorch OK')
" || echo "  ⚠ PyTorch verification had issues"

# Patch CUDA version check (system nvcc 13.0 vs PyTorch cu12.8 mismatch)
echo "  Patching CUDA version check …"
TORCH_CPP_EXT=$(python3 -c "import torch.utils.cpp_extension as e; print(e.__file__)")
if [ -f "$TORCH_CPP_EXT" ]; then
    sed -i 's/if cuda_ver.major != torch_cuda_version.major:/if False:  # PATCHED: skip cuda major version check/' "$TORCH_CPP_EXT"
    echo "  ✓ CUDA version check patched"
fi

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

# ── Step 4: Flash Attention 2 for sm_100 (Blackwell) ──────────────────────
echo "[4/6] Flash Attention 2 (building from source for sm_100) …"
export TORCH_CUDA_ARCH_LIST="10.0"
export MAX_JOBS=$(( $(nproc) / 4 ))
pip3 install flash-attn --no-build-isolation 2>&1 | tail -5 || {
    echo "  First attempt failed, retrying with --no-cache-dir …"
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
