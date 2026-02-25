#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# ApexIntel — Full Training, Evaluation & Export Pipeline
# Optimized for 8×RTX 5090 (32 GB each, 256 GB total) — DeepSpeed ZeRO-3
#
# Model: Qwen3-30B-A3B (30B total / 3B active MoE) — LoRA fine-tuning
#
# Orchestrates:
#   0. Pre-flight validation
#   1. Training data preparation
#   2. Eval data generation (654+ scenarios)
#   3. Eval dry-run
#   4. Phase 1: Domain-Adaptive Pre-Training (DAPT) — 8×5090
#   5. Phase 2: Supervised Fine-Tuning (SFT) — 8×5090
#   6. LLM stress test (8 curated domain questions)
#   7. Evaluation harness (500+ test scenarios)
#   8. Model export (merge adapters + optional GGUF)
#   9. Disk cleanup
#
# Environment variables:
#   WORK_DIR        — workspace root (default: /workspace/ApexIntel)
#   SKIP_VALIDATE   — skip pre-flight validation (default: 0)
#   SKIP_DAPT       — skip Phase 1 DAPT (default: 0)
#   SKIP_SFT        — skip Phase 2 SFT (default: 0)
#   SKIP_EVAL       — skip evaluation (default: 0)
#   SKIP_EXPORT     — skip model export (default: 0)
#   GGUF_QUANT      — GGUF quantization type (default: none; e.g. Q4_K_M)
#   EVAL_MAX        — limit eval examples per file (default: 0 = all)
#   HF_REPO         — push merged model to HF Hub (default: none)
#   LOG_DIR         — log directory (default: $WORK_DIR/training/outputs/logs)
#
# Usage:
#   bash training/run_all.sh                              # full pipeline
#   SKIP_DAPT=1 SKIP_SFT=1 bash training/run_all.sh      # eval + export only
#   GGUF_QUANT=Q4_K_M bash training/run_all.sh            # with GGUF export
#   EVAL_MAX=10 bash training/run_all.sh                  # quick smoke test
# ═══════════════════════════════════════════════════════════════════
set -euo pipefail

WORK_DIR="${WORK_DIR:-/workspace/ApexIntel}"
SKIP_VALIDATE="${SKIP_VALIDATE:-0}"
SKIP_DAPT="${SKIP_DAPT:-0}"
SKIP_SFT="${SKIP_SFT:-0}"
SKIP_EVAL="${SKIP_EVAL:-0}"
SKIP_EXPORT="${SKIP_EXPORT:-0}"
GGUF_QUANT="${GGUF_QUANT:-}"
EVAL_MAX="${EVAL_MAX:-0}"
HF_REPO="${HF_REPO:-}"
LOG_DIR="${LOG_DIR:-$WORK_DIR/training/outputs/logs}"

cd "$WORK_DIR"
mkdir -p "$LOG_DIR"

# ── Multi-GPU / NCCL performance tuning (8×RTX 5090, PCIe, cross-NUMA) ────────
export NCCL_P2P_LEVEL=SYS                     # PCIe cross-NUMA (no NVLink)
export NCCL_IB_DISABLE=1                      # no InfiniBand
export NCCL_DEBUG=WARN                        # only warnings
export NCCL_ALGO=Ring                         # Ring optimal for 8 GPUs PCIe
export NCCL_MAX_NCHANNELS=8                   # parallel comm channels
export NCCL_MIN_NCHANNELS=4

# ── PyTorch memory allocation tuning ──────────────────────────────────────────
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True,garbage_collection_threshold:0.8
export NCCL_BUFFSIZE=4194304                  # 4MB comm buffer (PCIe friendly)
export NCCL_NET_GDR_LEVEL=0                   # no GPU Direct RDMA on PCIe
export CUDA_DEVICE_MAX_CONNECTIONS=1           # optimize per-stream scheduling
export TORCH_NCCL_ASYNC_ERROR_HANDLING=1       # catch NCCL errors early
export OMP_NUM_THREADS=24                      # 192 cores / 8 GPUs = 24
export TOKENIZERS_PARALLELISM=false            # avoid tokenizer fork deadlocks
export CUDA_LAUNCH_BLOCKING=0                  # async kernel launches
export TORCH_CUDA_ALLOC_CONF=expandable_segments:True  # reduce fragmentation
export PYTORCH_CUDA_ALLOC_CONF=expandable_segments:True

TIMESTAMP=$(date +%Y%m%d_%H%M%S)
LOGFILE="$LOG_DIR/pipeline_${TIMESTAMP}.log"

# ── Helpers ─────────────────────────────────────────────────────────────────

log() { echo "[$(date +%H:%M:%S)] $*" | tee -a "$LOGFILE"; }
step_start() { STEP_T0=$(date +%s); log ""; log "═══ $1 ═══"; }
step_done() {
    local elapsed=$(( $(date +%s) - STEP_T0 ))
    log "  ✓ $1 (${elapsed}s)"
}
die() { log "  ✗ FATAL: $1"; exit 1; }

PIPELINE_T0=$(date +%s)
log "═══════════════════════════════════════════════════════════"
log "  ApexIntel Training Pipeline  (8×RTX 5090, Qwen3-30B-A3B)"
log "  Started: $(date)"
log "  Workspace: $WORK_DIR"
log "  Log: $LOGFILE"
log "═══════════════════════════════════════════════════════════"

# ── Step 0: Pre-flight validation ──────────────────────────────────────────

if [ "$SKIP_VALIDATE" = "0" ]; then
    step_start "[0/9] Pre-flight validation"
    python3 training/validate_pipeline.py --work-dir "$WORK_DIR" 2>&1 | tee -a "$LOGFILE"
    step_done "Validation passed"

    # Verify critical optimizations
    log "  Checking optimizations …"
    python3 -c "
import torch
print(f'  PyTorch {torch.__version__}, CUDA {torch.version.cuda}')
n = torch.cuda.device_count()
print(f'  GPUs: {n}')
for i in range(n):
    name = torch.cuda.get_device_name(i)
    mem = torch.cuda.get_device_properties(i).total_memory / (1024**3)
    print(f'    GPU {i}: {name} ({mem:.0f} GB)')
print(f'  TF32 matmul: {torch.backends.cuda.matmul.allow_tf32}')

try:
    import flash_attn
    from flash_attn.flash_attn_interface import flash_attn_func
    print(f'  ✓ Flash Attention 2 v{flash_attn.__version__} — CUDA kernels OK')
except Exception:
    print('  ⚠ flash-attn not available — using SDPA fallback')

try:
    from causal_conv1d import causal_conv1d_fn
    print('  ✓ causal_conv1d — CUDA kernels OK')
except Exception as e:
    print(f'  ⚠ causal_conv1d: {e}')

try:
    import fla
    print('  ✓ flash-linear-attention OK')
except:
    pass
" 2>&1 | tee -a "$LOGFILE"
    step_done "Optimization check passed"
else
    log "[0/9] Validation SKIPPED"
fi

# ── Start GPU monitor in background ────────────────────────────────────────
log "Starting GPU utilization monitor …"
bash training/gpu_monitor.sh 30 "$LOG_DIR/gpu_utilization.csv" > "$LOG_DIR/gpu_monitor.out" 2>&1 &
GPU_MON_PID=$!
log "  GPU monitor PID: $GPU_MON_PID"

# ── Step 1: Prepare training data ─────────────────────────────────────────

step_start "[1/9] Prepare training data"
python3 training/prepare_data.py --work-dir "$WORK_DIR" 2>&1 | tee -a "$LOGFILE"
step_done "Training data prepared"

# ── Step 2: Generate eval scenarios ────────────────────────────────────────

step_start "[2/9] Generate eval scenarios (654+)"
python3 training_data/evaluation/generate_eval_data.py \
    --recipe-count 200 \
    --poi-count 120 \
    --memo-count 60 \
    --entity-count 250 2>&1 | tee -a "$LOGFILE"
step_done "Eval scenarios generated"

# ── Step 3: Eval dry-run (validate eval data) ─────────────────────────────

step_start "[3/9] Eval data dry-run validation"
python3 training/eval_harness.py --dry-run \
    --out-report "$WORK_DIR/training/outputs/eval_dryrun.json" 2>&1 | tee -a "$LOGFILE"
step_done "Eval data validated"

# ── Step 4: Phase 1 DAPT ──────────────────────────────────────────────────

if [ "$SKIP_DAPT" = "0" ]; then
    step_start "[4/9] Phase 1: Domain-Adaptive Pre-Training (8×5090)"
    log "  Effective batch: 1×8×8 = 64 | seq_len: 4096 | epochs: 1 | QLoRA 4-bit NF4 | DDP"
    accelerate launch \
        --config_file training/configs/accelerate_8gpu.yaml \
        training/train_phase1_dapt.py 2>&1 | tee -a "$LOGFILE"
    step_done "Phase 1 DAPT complete"
else
    log "[4/9] Phase 1 DAPT SKIPPED"
fi

# ── Step 5: Phase 2 SFT ───────────────────────────────────────────────────

if [ "$SKIP_SFT" = "0" ]; then
    # Pre-merge Phase 1 adapter into base model (single-process, CPU)
    # This avoids OOM when ZeRO-3 tries to move the full 30B model to GPU
    step_start "[5a/9] Pre-merge Phase 1 adapter"
    python3 training/pre_merge_phase1.py 2>&1 | tee -a "$LOGFILE"
    step_done "Phase 1 adapter merge complete"

    step_start "[5b/9] Phase 2: Supervised Fine-Tuning (8×5090)"
    log "  Effective batch: 1×8×8 = 64 | seq_len: 2048 | epochs: 3 | SDPA + NEFTune | FSDP FULL_SHARD"
    accelerate launch \
        --config_file training/configs/accelerate_fsdp_8gpu.yaml \
        training/train_phase2_sft.py 2>&1 | tee -a "$LOGFILE"
    step_done "Phase 2 SFT complete"
else
    log "[5/9] Phase 2 SFT SKIPPED"
fi

# ── Step 6: LLM Stress Test (post-training quality check) ─────────────────

step_start "[6/9] LLM stress test (8 curated domain questions)"
python3 training/llm_stress_test.py \
    --out-report "$WORK_DIR/training/outputs/stress_test_report.json" 2>&1 | tee -a "$LOGFILE" || {
    LLM_EXIT=$?
    log "  ⚠ LLM stress test exited $LLM_EXIT — some tests may have failed"
    log "  See training/outputs/stress_test_report.json for details"
}
step_done "LLM stress test done"

# ── Step 7: Evaluation ────────────────────────────────────────────────────

if [ "$SKIP_EVAL" = "0" ]; then
    step_start "[7/9] Evaluation (654+ scenarios)"
    EVAL_ARGS="--out-report $WORK_DIR/training/outputs/eval_report.json"
    if [ "$EVAL_MAX" != "0" ]; then
        EVAL_ARGS="$EVAL_ARGS --max-examples $EVAL_MAX"
        log "  (limited to $EVAL_MAX examples per file)"
    fi
    python3 training/eval_harness.py $EVAL_ARGS 2>&1 | tee -a "$LOGFILE"
    step_done "Evaluation complete"
else
    log "[7/9] Evaluation SKIPPED"
fi

# ── Step 8: Export ─────────────────────────────────────────────────────────

if [ "$SKIP_EXPORT" = "0" ]; then
    step_start "[8/9] Model export (merge adapters)"
    EXPORT_ARGS="--merge"
    if [ -n "$GGUF_QUANT" ]; then
        EXPORT_ARGS="$EXPORT_ARGS --gguf $GGUF_QUANT"
        log "  GGUF quantization: $GGUF_QUANT"
    fi
    if [ -n "$HF_REPO" ]; then
        EXPORT_ARGS="$EXPORT_ARGS --push --repo $HF_REPO"
        log "  Will push to HuggingFace: $HF_REPO"
    fi
    python3 training/export_model.py $EXPORT_ARGS 2>&1 | tee -a "$LOGFILE"
    step_done "Export complete"
else
    log "[8/9] Export SKIPPED"
fi

# ── Step 9: Disk cleanup ──────────────────────────────────────────────────

step_start "[9/9] Disk cleanup"
# Remove HF cache if present
rm -rf /root/.cache/huggingface/hub 2>/dev/null || true
# Remove old checkpoints (keep only best_adapter)
for phase_dir in training/outputs/phase1_dapt training/outputs/phase2_sft; do
    if [ -d "$WORK_DIR/$phase_dir" ]; then
        find "$WORK_DIR/$phase_dir" -maxdepth 1 -name "checkpoint-*" -type d -exec rm -rf {} + 2>/dev/null || true
    fi
done
log "  Disk after cleanup:"
df -h /workspace 2>/dev/null | tail -1 | awk '{print "    " $3 " used / " $2 " total (" $5 " used)"}' | tee -a "$LOGFILE"
step_done "Cleanup complete"

# ── Stop GPU monitor ──────────────────────────────────────────────────────
if [ -n "${GPU_MON_PID:-}" ]; then
    kill "$GPU_MON_PID" 2>/dev/null || true
    log "  GPU monitor stopped"
fi

# ── Summary ────────────────────────────────────────────────────────────────

PIPELINE_ELAPSED=$(( $(date +%s) - PIPELINE_T0 ))
HOURS=$(( PIPELINE_ELAPSED / 3600 ))
MINUTES=$(( (PIPELINE_ELAPSED % 3600) / 60 ))
SECONDS=$(( PIPELINE_ELAPSED % 60 ))

log ""
log "═══════════════════════════════════════════════════════════"
log "  Pipeline complete!"
log "  Total time: ${HOURS}h ${MINUTES}m ${SECONDS}s"
log "  Outputs: $WORK_DIR/training/outputs/"
log "  Log: $LOGFILE"
log "═══════════════════════════════════════════════════════════"

# List output artifacts
log ""
log "  Artifacts:"
if [ -d "$WORK_DIR/training/outputs/phase1_dapt" ]; then
    log "    ✓ Phase 1 adapter: training/outputs/phase1_dapt/best_adapter/"
fi
if [ -d "$WORK_DIR/training/outputs/phase2_sft" ]; then
    log "    ✓ Phase 2 adapter: training/outputs/phase2_sft/best_adapter/"
fi
if [ -d "$WORK_DIR/training/outputs/merged" ]; then
    log "    ✓ Merged model: training/outputs/merged/"
fi
if [ -d "$WORK_DIR/training/outputs/gguf" ]; then
    log "    ✓ GGUF: training/outputs/gguf/"
fi
if [ -f "$WORK_DIR/training/outputs/stress_test_report.json" ]; then
    log "    ✓ Stress test: training/outputs/stress_test_report.json"
    STRESS_RATE=$(python3 -c "import json; r=json.load(open('$WORK_DIR/training/outputs/stress_test_report.json')); print(f\"{r.get('pass_rate', '?')}%\")" 2>/dev/null || echo "?")
    log "    ✓ Stress test pass rate: $STRESS_RATE"
fi
if [ -f "$WORK_DIR/training/outputs/eval_report.json" ]; then
    log "    ✓ Eval report: training/outputs/eval_report.json"
    # Print pass rate from report
    PASS_RATE=$(python3 -c "import json; r=json.load(open('$WORK_DIR/training/outputs/eval_report.json')); print(f\"{r.get('pass_rate', '?')}%\")" 2>/dev/null || echo "?")
    log "    ✓ Eval pass rate: $PASS_RATE"
fi
log ""
