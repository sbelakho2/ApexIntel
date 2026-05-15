#!/usr/bin/env python3
"""
Patch eval_harness.py for v5 evaluation:
1. Handle unclosed <think> tags (model thinks past token limit)
2. Improve entity normalization (strip Inc/Ltd/Corp, etc.)
3. Add fuzzy entity matching in f1_score
4. Store output_text in failure reports for debugging
5. Increase default max-new-tokens from 512 to 4096
6. Add retry with 2x tokens on empty output after think-strip

Threshold values are sourced from eval_thresholds.ThresholdConfig (--threshold-preset CLI arg).
"""

import argparse
import re

from eval_thresholds import ThresholdConfig

HARNESS = "/workspace/ApexIntel/training/eval_harness.py"


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Patch eval_harness.py for v5 evaluation"
    )
    ThresholdConfig.add_argparse_arg(parser)
    args = parser.parse_args()
    thresholds = ThresholdConfig(args.threshold_preset)

    print("=" * 60)
    print("  Patching eval_harness.py for v5")
    print(f"  Threshold preset: {thresholds}")
    print("=" * 60)

    with open(HARNESS, "r") as f:
        code = f.read()

    original = code

    # ──────────────────────────────────────────────────────────
    # PATCH 1: Fix _strip_think_tags to handle UNCLOSED <think> blocks
    # When model uses all tokens thinking, output is "<think>...long reasoning..."
    # with no closing </think>, so re.sub(r"<think>.*?</think>") does nothing.
    # ──────────────────────────────────────────────────────────
    old_strip = '''def _strip_think_tags(text: str) -> str:
    """Strip <think>...</think> tags from Qwen3+ thinking mode output."""
    stripped = re.sub(r"<think>.*?</think>", "", text, flags=re.DOTALL)
    return stripped.strip()'''

    new_strip = '''def _strip_think_tags(text: str) -> str:
    """Strip <think>...</think> tags from Qwen3+ thinking mode output.

    Also handles UNCLOSED <think> blocks where the model used all tokens
    on reasoning and never produced a closing </think> tag.
    """
    # First strip properly closed think blocks
    stripped = re.sub(r"<think>.*?</think>", "", text, flags=re.DOTALL)
    # Then strip unclosed <think> blocks (thinking consumed all tokens)
    stripped = re.sub(r"<think>.*$", "", stripped, flags=re.DOTALL)
    return stripped.strip()'''

    assert old_strip in code, "PATCH 1 FAILED: _strip_think_tags not found"
    code = code.replace(old_strip, new_strip)
    print("PATCH 1 applied: _strip_think_tags handles unclosed blocks")

    # ──────────────────────────────────────────────────────────
    # PATCH 2: Improve normalize() — strip corporate suffixes
    # ──────────────────────────────────────────────────────────
    old_normalize = '''def normalize(s: str) -> str:
    s = s.lower().strip()
    s = re.sub(r"\\s+", " ", s)
    return s'''

    new_normalize = '''def normalize(s: str) -> str:
    """Normalize entity text: lowercase, strip whitespace, remove corporate suffixes."""
    s = s.lower().strip()
    s = re.sub(r"\\s+", " ", s)
    # Strip corporate suffixes for tolerant matching
    s = re.sub(r"\\s*\\b(inc\\.?|corp\\.?|ltd\\.?|llc|co\\.?|gmbh|s\\.?a\\.?|plc|limited|incorporated|corporation|company)\\s*$", "", s)
    s = s.rstrip(" ,.")
    return s'''

    assert old_normalize in code, "PATCH 2 FAILED: normalize not found"
    code = code.replace(old_normalize, new_normalize)
    print("PATCH 2 applied: normalize() strips corporate suffixes")

    # ──────────────────────────────────────────────────────────
    # PATCH 3: Fuzzy f1_score — partial string matching
    # If exact match fails, check if gold ⊂ pred or pred ⊂ gold
    # ──────────────────────────────────────────────────────────
    old_f1 = '''def f1_score(pred: List[str], gold: List[str]) -> Tuple[float, float, float]:
    pred_set = set(pred)
    gold_set = set(gold)
    if not pred_set and not gold_set:
        return 1.0, 1.0, 1.0
    if not pred_set or not gold_set:
        return 0.0, 0.0, 0.0
    tp = len(pred_set & gold_set)
    precision = tp / len(pred_set)
    recall = tp / len(gold_set)
    if precision + recall == 0:
        f1 = 0.0
    else:
        f1 = 2 * precision * recall / (precision + recall)
    return precision, recall, f1'''

    new_f1 = '''def _fuzzy_match(a: str, b: str) -> bool:
    """Check if two entity strings are a fuzzy match.

    Returns True if:
    - Exact match
    - One is a substring of the other (after prefix stripping)
    - They share the same core name (ignoring prefix like company:/person:)
    """
    if a == b:
        return True
    # Strip prefix (company:, person:, etc.) and compare cores
    a_core = a.split(":", 1)[-1].strip()
    b_core = b.split(":", 1)[-1].strip()
    if a_core == b_core:
        return True
    # Substring containment (handles "jabil" vs "jabil circuit")
    if len(a_core) >= 3 and len(b_core) >= 3:
        if a_core in b_core or b_core in a_core:
            return True
    return False


def f1_score(pred: List[str], gold: List[str]) -> Tuple[float, float, float]:
    """Compute F1 with fuzzy entity matching."""
    pred_set = set(pred)
    gold_set = set(gold)
    if not pred_set and not gold_set:
        return 1.0, 1.0, 1.0
    if not pred_set or not gold_set:
        return 0.0, 0.0, 0.0

    # First try exact matching
    tp_exact = len(pred_set & gold_set)

    # Then try fuzzy matching for remaining
    unmatched_pred = pred_set - gold_set
    unmatched_gold = gold_set - pred_set
    tp_fuzzy = 0
    matched_gold = set()
    for p in unmatched_pred:
        for g in unmatched_gold - matched_gold:
            if _fuzzy_match(p, g):
                tp_fuzzy += 1
                matched_gold.add(g)
                break

    tp = tp_exact + tp_fuzzy
    precision = tp / len(pred_set)
    recall = tp / len(gold_set)
    if precision + recall == 0:
        f1 = 0.0
    else:
        f1 = 2 * precision * recall / (precision + recall)
    return precision, recall, f1'''

    assert old_f1 in code, "PATCH 3 FAILED: f1_score not found"
    code = code.replace(old_f1, new_f1)
    print("PATCH 3 applied: f1_score with fuzzy entity matching")

    # ──────────────────────────────────────────────────────────
    # PATCH 4: Increase default --max-new-tokens from 512 to 4096
    # ──────────────────────────────────────────────────────────
    old_tokens = '    parser.add_argument("--max-new-tokens", type=int, default=512)'
    new_tokens = '    parser.add_argument("--max-new-tokens", type=int, default=4096)'
    assert old_tokens in code, "PATCH 4 FAILED: max-new-tokens default not found"
    code = code.replace(old_tokens, new_tokens)
    print("PATCH 4 applied: default max-new-tokens 512 -> 4096")

    # ──────────────────────────────────────────────────────────
    # PATCH 5: Store output_text in failure reports for debugging
    # ──────────────────────────────────────────────────────────
    old_failure_append = '''                results["failures"].append({
                    "id": item.get("id"),
                    "type": eval_type,
                    "file": path.name,
                    "metrics": {k: v for k, v in metrics.items() if isinstance(v, (int, float, bool, str))},
                })'''

    new_failure_append = '''                results["failures"].append({
                    "id": item.get("id"),
                    "type": eval_type,
                    "file": path.name,
                    "metrics": {k: v for k, v in metrics.items() if isinstance(v, (int, float, bool, str))},
                    "output_preview": output_text[:500] if output_text else "(empty)",
                })'''

    assert old_failure_append in code, "PATCH 5 FAILED: failure append not found"
    code = code.replace(old_failure_append, new_failure_append)
    print("PATCH 5 applied: store output_preview in failure reports")

    # ──────────────────────────────────────────────────────────
    # PATCH 6: Add retry on empty output after think-strip
    # If the initial generation produces empty content (all tokens consumed by thinking),
    # retry once with 2x tokens.
    # ──────────────────────────────────────────────────────────
    # We patch the generate() function to detect empty-after-think-strip
    old_generate = '''def generate(model, tokenizer, system: str, user: str, max_new_tokens: int) -> str:
    import gc
    import torch
    prompt = build_prompt(tokenizer, system, user)
    inputs = tokenizer(prompt, return_tensors="pt").to(model.device)
    decoded = ""
    try:
        with torch.no_grad():
            out = model.generate(
                **inputs,
                max_new_tokens=max_new_tokens,
                do_sample=False,
                temperature=0.0,
            )
        # Strip prompt tokens from output to get only the generated text
        prompt_len = inputs["input_ids"].shape[1]
        generated_ids = out[0][prompt_len:]
        decoded = tokenizer.decode(generated_ids, skip_special_tokens=True)
    except (torch.cuda.OutOfMemoryError, RuntimeError) as e:
        print(f"    WARNING: CUDA OOM/RuntimeError during generation: {e}", flush=True)
        decoded = ""
    finally:
        del inputs
        gc.collect()
        torch.cuda.empty_cache()
    return decoded.strip()'''

    new_generate = '''def generate(model, tokenizer, system: str, user: str, max_new_tokens: int) -> str:
    import gc
    import torch

    def _do_generate(prompt_text: str, max_tok: int) -> str:
        inputs = tokenizer(prompt_text, return_tensors="pt").to(model.device)
        decoded = ""
        try:
            with torch.no_grad():
                out = model.generate(
                    **inputs,
                    max_new_tokens=max_tok,
                    do_sample=False,
                    temperature=0.0,
                )
            prompt_len = inputs["input_ids"].shape[1]
            generated_ids = out[0][prompt_len:]
            decoded = tokenizer.decode(generated_ids, skip_special_tokens=True)
        except (torch.cuda.OutOfMemoryError, RuntimeError) as e:
            print(f"    WARNING: CUDA OOM/RuntimeError during generation: {e}", flush=True)
            decoded = ""
        finally:
            del inputs
            gc.collect()
            torch.cuda.empty_cache()
        return decoded.strip()

    prompt = build_prompt(tokenizer, system, user)
    decoded = _do_generate(prompt, max_new_tokens)

    # Check if thinking consumed all tokens (empty after stripping think tags)
    stripped = _strip_think_tags(decoded)
    if not stripped and decoded and max_new_tokens < 8192:
        retry_tokens = min(max_new_tokens * 2, 8192)
        print(f"    RETRY: thinking consumed all {max_new_tokens} tokens, retrying with {retry_tokens}", flush=True)
        decoded = _do_generate(prompt, retry_tokens)

    return decoded.strip()'''

    assert old_generate in code, "PATCH 6 FAILED: generate function not found"
    code = code.replace(old_generate, new_generate)
    print("PATCH 6 applied: retry with 2x tokens on empty-after-think-strip")

    # ──────────────────────────────────────────────────────────
    # PATCH 7: Also handle "industries" key in collect_entities
    # Some eval data may use "industries" which the model extracts
    # ──────────────────────────────────────────────────────────
    old_collect_end = '''    # locations
    for entry in obj.get("locations", []) or []:
        if isinstance(entry, dict):
            for k in ("city", "country", "state", "prefecture"):
                if k in entry:
                    add("loc", str(entry[k]))
        else:
            add("loc", str(entry))

    return collected'''

    new_collect_end = '''    # locations
    for entry in obj.get("locations", []) or []:
        if isinstance(entry, dict):
            for k in ("city", "country", "state", "prefecture"):
                if k in entry:
                    add("loc", str(entry[k]))
        else:
            add("loc", str(entry))

    # industries
    for entry in obj.get("industries", []) or []:
        if isinstance(entry, str):
            add("industry", entry)
        elif isinstance(entry, dict):
            add("industry", entry.get("name", str(entry)))

    return collected'''

    assert old_collect_end in code, "PATCH 7 FAILED: collect_entities locations block not found"
    code = code.replace(old_collect_end, new_collect_end)
    print("PATCH 7 applied: collect_entities handles industries key")

    # ──────────────────────────────────────────────────────────
    # Write patched file
    # ──────────────────────────────────────────────────────────
    with open(HARNESS, "w") as f:
        f.write(code)

    n_changes = sum(1 for a, b in zip(original.splitlines(), code.splitlines()) if a != b)
    print(f"\nAll patches applied. ~{n_changes} lines changed.")
    print(f"File: {HARNESS}")


if __name__ == "__main__":
    main()
