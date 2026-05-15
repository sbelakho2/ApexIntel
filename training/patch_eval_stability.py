#!/usr/bin/env python3
"""Patch eval_harness.py for multi-GPU stability.

Adds:
1. CUDA OOM handling in generate() with graceful recovery
2. torch.cuda.empty_cache() + gc.collect() after each generation
3. try/except wrapper in main eval loop

Threshold values are sourced from eval_thresholds.ThresholdConfig (--threshold-preset CLI arg).
"""
import argparse
import re

from eval_thresholds import ThresholdConfig

HARNESS = "/workspace/ApexIntel/training/eval_harness.py"


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Patch eval_harness.py for multi-GPU stability"
    )
    ThresholdConfig.add_argparse_arg(parser)
    args = parser.parse_args()
    thresholds = ThresholdConfig(args.threshold_preset)

    print("=" * 60)
    print("  Patching eval_harness.py for multi-GPU stability")
    print(f"  Threshold preset: {thresholds}")
    print("=" * 60)

    with open(HARNESS, "r") as f:
        code = f.read()

    patches_applied = 0

    # ── PATCH 1: Replace generate() with OOM-safe version ──
    old_gen = (
        'def generate(model, tokenizer, system: str, user: str, max_new_tokens: int) -> str:\n'
        '    import torch\n'
        '    prompt = build_prompt(tokenizer, system, user)\n'
        '    inputs = tokenizer(prompt, return_tensors="pt").to(model.device)\n'
        '    with torch.no_grad():\n'
        '        out = model.generate(\n'
        '            **inputs,\n'
        '            max_new_tokens=max_new_tokens,\n'
        '            do_sample=False,\n'
        '            temperature=0.0,\n'
        '        )\n'
        '    # Strip prompt tokens from output to get only the generated text\n'
        '    prompt_len = inputs["input_ids"].shape[1]\n'
        '    generated_ids = out[0][prompt_len:]\n'
        '    decoded = tokenizer.decode(generated_ids, skip_special_tokens=True)\n'
        '    return decoded.strip()'
    )

    new_gen = (
        'def generate(model, tokenizer, system: str, user: str, max_new_tokens: int) -> str:\n'
        '    import gc\n'
        '    import torch\n'
        '    prompt = build_prompt(tokenizer, system, user)\n'
        '    inputs = tokenizer(prompt, return_tensors="pt").to(model.device)\n'
        '    decoded = ""\n'
        '    try:\n'
        '        with torch.no_grad():\n'
        '            out = model.generate(\n'
        '                **inputs,\n'
        '                max_new_tokens=max_new_tokens,\n'
        '                do_sample=False,\n'
        '                temperature=0.0,\n'
        '            )\n'
        '        # Strip prompt tokens from output to get only the generated text\n'
        '        prompt_len = inputs["input_ids"].shape[1]\n'
        '        generated_ids = out[0][prompt_len:]\n'
        '        decoded = tokenizer.decode(generated_ids, skip_special_tokens=True)\n'
        '    except (torch.cuda.OutOfMemoryError, RuntimeError) as e:\n'
        '        print(f"    WARNING: CUDA OOM/RuntimeError during generation: {e}", flush=True)\n'
        '        decoded = ""\n'
        '    finally:\n'
        '        del inputs\n'
        '        gc.collect()\n'
        '        torch.cuda.empty_cache()\n'
        '    return decoded.strip()'
    )

    if old_gen in code:
        code = code.replace(old_gen, new_gen)
        patches_applied += 1
        print("PATCH 1: generate() OOM handling + empty_cache - APPLIED")
    else:
        print("PATCH 1: generate() - NOT FOUND (may already be patched)")

    # ── PATCH 2: Wrap main loop's generate call in try/except ──
    old_main = (
        '            t1 = time.time()\n'
        '            output_text = generate(model, tokenizer, system, user, args.max_new_tokens)\n'
        '            gen_time = time.time() - t1'
    )

    new_main = (
        '            t1 = time.time()\n'
        '            try:\n'
        '                output_text = generate(model, tokenizer, system, user, args.max_new_tokens)\n'
        '            except Exception as gen_err:\n'
        '                import traceback\n'
        '                print(f"    FATAL generation error: {gen_err}", flush=True)\n'
        '                traceback.print_exc()\n'
        '                output_text = ""\n'
        '            gen_time = time.time() - t1'
    )

    if old_main in code:
        code = code.replace(old_main, new_main)
        patches_applied += 1
        print("PATCH 2: main loop generate try/except - APPLIED")
    else:
        print("PATCH 2: main loop generate - NOT FOUND")

    # Write patched file
    with open(HARNESS, "w") as f:
        f.write(code)

    print(f"\nDone. {patches_applied} patches applied.")


if __name__ == "__main__":
    main()
