#!/bin/bash
set -e

echo "=== Starting builds at $(date) ==="

# Build flash-attn for sm_100 only
echo "=== Building flash-attn ==="
cd /root/flash-attention
export FLASH_ATTN_CUDA_ARCHS="100"
export MAX_JOBS=48
export FLASH_ATTENTION_FORCE_BUILD=TRUE
echo "FLASH_ATTN_CUDA_ARCHS=$FLASH_ATTN_CUDA_ARCHS MAX_JOBS=$MAX_JOBS"

python3 -c "
import os
archs = os.getenv('FLASH_ATTN_CUDA_ARCHS', '80;90;100;110;120').split(';')
print(f'Flash-attn building for archs: {archs}')
assert '100' in archs, 'sm_100 not in archs!'
"

pip install . --no-build-isolation
echo "=== flash-attn EXIT_CODE=$? ==="

# Test flash-attn
python3 -c "
import flash_attn
print(f'flash_attn version: {flash_attn.__version__}')
from flash_attn.flash_attn_interface import flash_attn_func
print('flash_attn CUDA kernels: OK')
"

# Build causal-conv1d for sm_100
echo "=== Building causal-conv1d ==="
cd /root/causal-conv1d
export TORCH_CUDA_ARCH_LIST="10.0"
export CAUSAL_CONV1D_FORCE_BUILD=TRUE
export MAX_JOBS=32
echo "TORCH_CUDA_ARCH_LIST=$TORCH_CUDA_ARCH_LIST MAX_JOBS=$MAX_JOBS"

pip install . --no-build-isolation
echo "=== causal-conv1d EXIT_CODE=$? ==="

# Test causal-conv1d
python3 -c "
from causal_conv1d import causal_conv1d_fn
print('causal_conv1d CUDA kernels: OK')
"

echo "=== All builds complete at $(date) ==="
