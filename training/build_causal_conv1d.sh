#!/bin/bash
set -ex
cd /root/causal-conv1d
export TORCH_CUDA_ARCH_LIST="10.0"
export CAUSAL_CONV1D_FORCE_BUILD=TRUE
export MAX_JOBS=32

echo "TORCH_CUDA_ARCH_LIST=$TORCH_CUDA_ARCH_LIST"

pip install . --no-build-isolation 2>&1
echo "EXIT_CODE=$?"
