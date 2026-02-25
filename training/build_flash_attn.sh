#!/bin/bash
set -ex
cd /root/flash-attention
export FLASH_ATTN_CUDA_ARCHS="100"
export MAX_JOBS=64
export FLASH_ATTENTION_FORCE_BUILD=TRUE

echo "FLASH_ATTN_CUDA_ARCHS=$FLASH_ATTN_CUDA_ARCHS"
echo "MAX_JOBS=$MAX_JOBS"

# Verify the arch selection
python3 -c "
import os
archs = os.getenv('FLASH_ATTN_CUDA_ARCHS', '80;90;100;110;120').split(';')
print(f'Building for archs: {archs}')
assert archs == ['100'], f'Expected [100] but got {archs}'
"

pip install . --no-build-isolation 2>&1
echo "EXIT_CODE=$?"
