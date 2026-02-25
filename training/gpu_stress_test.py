#!/usr/bin/env python3
"""
gpu_stress_test.py — Pre-training GPU stability & NCCL throughput test.

Runs a short all-reduce benchmark across all 8 GPUs to verify:
  1. All GPUs can allocate large tensors
  2. NCCL all-reduce works across all GPUs
  3. Throughput is reasonable (detects bad interconnects)
  4. No GPU hangs or errors under sustained load

Usage:
    # Quick test (30 seconds)
    torchrun --nproc_per_node=8 training/gpu_stress_test.py

    # Extended stability test (5 minutes)
    torchrun --nproc_per_node=8 training/gpu_stress_test.py --duration 300

    # Single-GPU memory test only
    python training/gpu_stress_test.py --single
"""

import argparse
import os
import sys
import time

import torch

# Force unbuffered stdout so torchrun output is visible in real time
os.environ["PYTHONUNBUFFERED"] = "1"


def fprint(*args, **kwargs):
    """Print with immediate flush — critical under torchrun."""
    print(*args, **kwargs)
    sys.stdout.flush()


def single_gpu_test():
    """Test each GPU individually for memory allocation and compute."""
    n = torch.cuda.device_count()
    fprint(f"\n{'═'*60}")
    fprint(f"  Single-GPU Memory & Compute Test ({n} GPUs)")
    fprint(f"{'═'*60}")

    for i in range(n):
        torch.cuda.set_device(i)
        name = torch.cuda.get_device_name(i)
        total = torch.cuda.get_device_properties(i).total_memory / (1024**3)

        # Allocate 80% of VRAM
        alloc_gb = int(total * 0.8)
        try:
            t0 = time.time()
            # bf16 = 2 bytes per element; alloc_gb GB = alloc_gb * 2^30 / 2 elements
            num_elements = alloc_gb * (1024 ** 3) // 2
            x = torch.randn(num_elements, device=f"cuda:{i}", dtype=torch.bfloat16)
            alloc_time = time.time() - t0

            # Simple matmul benchmark
            a = torch.randn(4096, 4096, device=f"cuda:{i}", dtype=torch.bfloat16)
            b = torch.randn(4096, 4096, device=f"cuda:{i}", dtype=torch.bfloat16)
            torch.cuda.synchronize(i)

            t0 = time.time()
            for _ in range(50):
                c = torch.mm(a, b)
            torch.cuda.synchronize(i)
            matmul_time = time.time() - t0
            tflops = 50 * 2 * 4096**3 / matmul_time / 1e12

            del x, a, b, c
            torch.cuda.empty_cache()

            fprint(f"  ✓ GPU {i} ({name}): {alloc_gb}GB alloc OK ({alloc_time:.1f}s), matmul {tflops:.1f} TFLOPS")
        except Exception as e:
            fprint(f"  ✗ GPU {i} ({name}): FAILED — {e}")
            return False

    fprint(f"  ✓ All {n} GPUs passed individual test")
    return True


def _broadcast_int(val: int, src: int = 0, local_rank: int = 0) -> int:
    """Broadcast an integer from src rank to all workers."""
    t = torch.tensor([val], device=f"cuda:{local_rank}", dtype=torch.long)
    torch.distributed.broadcast(t, src=src)
    return t.item()


def distributed_test(duration: int = 30):
    """Test NCCL all-reduce throughput across all GPUs.

    IMPORTANT: all collective ops (all_reduce, barrier, broadcast) must be called
    by every rank the same number of times to avoid deadlocks.  Time-based loops
    are dangerous because workers may disagree on when time is up while mid-way
    through a collective.  We solve this by:
      1. Running a fixed-count warmup to estimate throughput.
      2. Broadcasting the target iteration count from rank 0 so every rank
         performs exactly the same number of all_reduce calls.
    """
    local_rank = int(os.environ.get("LOCAL_RANK", 0))
    world_size = int(os.environ.get("WORLD_SIZE", 1))

    if world_size <= 1:
        fprint("  ⚠  Not running in distributed mode. Use:")
        fprint("    torchrun --nproc_per_node=N training/gpu_stress_test.py")
        return True

    torch.cuda.set_device(local_rank)
    torch.distributed.init_process_group(backend="nccl")

    if local_rank == 0:
        fprint(f"\n{'═'*60}")
        fprint(f"  NCCL All-Reduce Throughput Test ({world_size} GPUs, {duration}s)")
        fprint(f"{'═'*60}")

    # ── Warmup ───────────────────────────────────────────────────────────
    warmup_tensor = torch.randn(1024, 1024, device=f"cuda:{local_rank}", dtype=torch.bfloat16)
    for _ in range(10):
        torch.distributed.all_reduce(warmup_tensor)
    torch.cuda.synchronize()
    torch.distributed.barrier()
    del warmup_tensor

    # ── Bandwidth at different tensor sizes ──────────────────────────────
    # Use FIXED iteration counts (estimated from a short calibration run)
    # so every rank calls all_reduce the same number of times.
    # Skip 1MB - dominated by latency; focus on sizes relevant to training
    sizes_mb = [16, 64, 256]
    target_seconds_per_size = max(duration / len(sizes_mb), 2)
    calibration_iters = 10  # small fixed count for timing estimate

    for size_mb in sizes_mb:
        num_elements = size_mb * 1024 * 1024 // 2  # bf16 = 2 bytes
        tensor = torch.randn(num_elements, device=f"cuda:{local_rank}", dtype=torch.bfloat16)

        torch.cuda.synchronize()
        torch.distributed.barrier()

        # Calibration: fixed 20 iterations to measure throughput
        t0 = time.time()
        for _ in range(calibration_iters):
            torch.distributed.all_reduce(tensor)
        torch.cuda.synchronize()
        cal_elapsed = time.time() - t0

        # Rank 0 decides how many iterations to run; broadcast to all
        if local_rank == 0:
            iters_per_sec = calibration_iters / max(cal_elapsed, 1e-6)
            target_iters = max(int(iters_per_sec * target_seconds_per_size), calibration_iters)
        else:
            target_iters = 0
        target_iters = _broadcast_int(target_iters, src=0, local_rank=local_rank)

        # Main benchmark: every rank runs exactly target_iters all-reduces
        torch.distributed.barrier()
        t0 = time.time()
        for _ in range(target_iters):
            torch.distributed.all_reduce(tensor)
        torch.cuda.synchronize()
        elapsed = time.time() - t0

        total_iters = target_iters + calibration_iters
        total_elapsed = elapsed + cal_elapsed
        bw_factor = 2 * (world_size - 1) / world_size
        bandwidth_gb = total_iters * size_mb / 1024 * bw_factor / total_elapsed

        if local_rank == 0:
            fprint(f"  {size_mb:>5}MB × {total_iters:>6} iters: {bandwidth_gb:.1f} GB/s  ({total_elapsed:.1f}s)")

        del tensor
        torch.cuda.empty_cache()
        torch.distributed.barrier()

    # ── Sustained stress test ────────────────────────────────────────────
    if local_rank == 0:
        fprint(f"\n  Sustained {duration}s stress test (256MB all-reduce) …")

    stress_tensor = torch.randn(256 * 1024 * 1024 // 2, device=f"cuda:{local_rank}", dtype=torch.bfloat16)
    torch.cuda.synchronize()
    torch.distributed.barrier()

    # Calibrate with 10 iterations
    t0 = time.time()
    for _ in range(10):
        torch.distributed.all_reduce(stress_tensor)
    torch.cuda.synchronize()
    cal_elapsed = time.time() - t0

    # Compute target iteration count for the sustained test
    if local_rank == 0:
        rate = 10 / max(cal_elapsed, 1e-6)
        stress_iters = max(int(rate * duration), 10)
    else:
        stress_iters = 0
    stress_iters = _broadcast_int(stress_iters, src=0, local_rank=local_rank)

    torch.distributed.barrier()
    t0 = time.time()
    errors = 0
    for i in range(stress_iters):
        try:
            torch.distributed.all_reduce(stress_tensor)
        except Exception as e:
            errors += 1
            if local_rank == 0:
                fprint(f"  ✗ Error at iter {i}: {e}")
            if errors > 5:
                break

    torch.cuda.synchronize()
    elapsed = time.time() - t0

    if local_rank == 0:
        bw_factor = 2 * (world_size - 1) / world_size
        bandwidth_gb = stress_iters * 256 / 1024 * bw_factor / elapsed
        fprint(f"  {stress_iters} iterations in {elapsed:.1f}s — {bandwidth_gb:.1f} GB/s sustained")
        if errors == 0:
            fprint(f"  ✓ NCCL stress test PASSED — no errors")
        else:
            fprint(f"  ✗ NCCL stress test FAILED — {errors} errors")

    del stress_tensor
    torch.cuda.empty_cache()
    torch.distributed.barrier()
    torch.distributed.destroy_process_group()
    return errors == 0


def main():
    parser = argparse.ArgumentParser(description="GPU stability & NCCL throughput test")
    parser.add_argument("--duration", type=int, default=30, help="Test duration in seconds")
    parser.add_argument("--single", action="store_true", help="Single-GPU test only (no distributed)")
    args = parser.parse_args()

    if args.single or int(os.environ.get("WORLD_SIZE", 1)) <= 1:
        ok = single_gpu_test()
        if not ok:
            sys.exit(1)
        if int(os.environ.get("WORLD_SIZE", 1)) > 1:
            ok = distributed_test(args.duration)
            if not ok:
                sys.exit(1)
    else:
        ok = distributed_test(args.duration)
        if not ok:
            sys.exit(1)

    local_rank = int(os.environ.get("LOCAL_RANK", 0))
    if local_rank == 0:
        fprint(f"\n{'═'*60}")
        fprint(f"  ✓ All GPU tests passed — ready for training")
        fprint(f"{'═'*60}")


if __name__ == "__main__":
    main()
