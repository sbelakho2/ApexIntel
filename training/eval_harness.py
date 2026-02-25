#!/usr/bin/env python3
"""Evaluate ApexIntel model on 500+ domain scenarios.

Runs all JSONL files in training_data/evaluation and produces a summary report.

Usage:
    # Full evaluation with model
    python training/eval_harness.py --model-dir ./models/base --adapter ./training/outputs/phase2_sft/best_adapter

    # Dry-run: validate eval data only (no model needed)
    python training/eval_harness.py --dry-run

    # Limit examples per file (for quick smoke tests)
    python training/eval_harness.py --max-examples 5
"""
import argparse
import json
import os
import re
import time
from pathlib import Path
from typing import Any, Dict, List, Tuple

WORK = Path(__file__).resolve().parent.parent

DEFAULT_SYSTEMS = {
    "entity_extraction": "Extract structured entities from this text.",
    "poi_synthesis": "You are synthesizing professional intelligence about a POI for an EMS competitive intelligence platform.",
    "recipe_hypothesis": "You are an OSINT analyst generating insight recipes for an EMS competitive intelligence platform.",
    "memo_quality": "You are writing a weekly strategy memo for an EMS General Manager.",
}


def extract_json(text: str) -> Any:
    text = text.strip()
    if not text:
        raise ValueError("empty text")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass

    # Try to extract the first JSON object/array from text
    obj_start = text.find("{")
    obj_end = text.rfind("}")
    arr_start = text.find("[")
    arr_end = text.rfind("]")

    candidates = []
    if obj_start != -1 and obj_end != -1 and obj_end > obj_start:
        candidates.append(text[obj_start:obj_end + 1])
    if arr_start != -1 and arr_end != -1 and arr_end > arr_start:
        candidates.append(text[arr_start:arr_end + 1])

    for cand in candidates:
        try:
            return json.loads(cand)
        except json.JSONDecodeError:
            continue

    raise ValueError("no valid json found")


def normalize(s: str) -> str:
    s = s.lower().strip()
    s = re.sub(r"\s+", " ", s)
    return s


def collect_entities(obj: Dict[str, Any]) -> List[str]:
    collected: List[str] = []

    def add(prefix: str, value: str):
        if value:
            collected.append(f"{prefix}:{normalize(value)}")

    # companies
    for entry in obj.get("companies", []) or []:
        if isinstance(entry, dict):
            add("company", entry.get("name", ""))
        else:
            add("company", str(entry))

    # persons
    for entry in obj.get("persons", []) or []:
        if isinstance(entry, dict):
            add("person", entry.get("name", ""))
        else:
            add("person", str(entry))

    # capabilities
    for entry in obj.get("capabilities", []) or []:
        add("capability", str(entry))

    # certifications
    for entry in obj.get("certifications", []) or []:
        add("cert", str(entry))

    # locations
    for entry in obj.get("locations", []) or []:
        if isinstance(entry, dict):
            for k in ("city", "country", "state", "prefecture"):
                if k in entry:
                    add("loc", str(entry[k]))
        else:
            add("loc", str(entry))

    return collected


def f1_score(pred: List[str], gold: List[str]) -> Tuple[float, float, float]:
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
    return precision, recall, f1


def build_prompt(tokenizer, system: str, user: str) -> str:
    messages = [
        {"role": "system", "content": system},
        {"role": "user", "content": user},
    ]
    return tokenizer.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)


def generate(model, tokenizer, system: str, user: str, max_new_tokens: int) -> str:
    import torch
    prompt = build_prompt(tokenizer, system, user)
    inputs = tokenizer(prompt, return_tensors="pt").to(model.device)
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
    return decoded.strip()


def load_model(model_dir: str, adapter_path: str | None = None):
    """Load model + optional adapter. Returns (model, tokenizer)."""
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from peft import PeftModel

    print(f"Loading model from {model_dir}")
    tokenizer = AutoTokenizer.from_pretrained(model_dir, trust_remote_code=True)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    model = AutoModelForCausalLM.from_pretrained(
        model_dir,
        torch_dtype=torch.bfloat16,
        device_map="auto",
        trust_remote_code=True,
    )

    if adapter_path and os.path.isdir(adapter_path):
        print(f"Loading adapter from {adapter_path}")
        model = PeftModel.from_pretrained(model, adapter_path)
        model = model.merge_and_unload()

    return model, tokenizer


def eval_recipe_quality(output_text: str, expected_schema: Dict[str, Any]) -> Dict[str, Any]:
    result = {"json_valid": False, "schema_ok": False, "score": 0.0}
    try:
        obj = extract_json(output_text)
        result["json_valid"] = True
    except Exception:
        return result

    required = expected_schema.get("required_fields", [])
    missing = [k for k in required if k not in obj]
    if missing:
        result["missing_fields"] = missing
        return result

    signals_min = expected_schema.get("signals_min", 1)
    if isinstance(obj.get("signals"), list) and len(obj.get("signals")) >= signals_min:
        result["schema_ok"] = True

    # Simple heuristic score
    result["score"] = 1.0 if result["json_valid"] and result["schema_ok"] else 0.0
    return result


def eval_entity_extraction(output_text: str, ground_truth: Dict[str, Any]) -> Dict[str, Any]:
    try:
        obj = extract_json(output_text)
    except Exception:
        return {"precision": 0.0, "recall": 0.0, "f1": 0.0}

    pred = collect_entities(obj if isinstance(obj, dict) else {})
    gold = collect_entities(ground_truth)
    precision, recall, f1 = f1_score(pred, gold)
    return {"precision": precision, "recall": recall, "f1": f1}


def eval_schema_only(output_text: str, expected_schema: Dict[str, Any]) -> Dict[str, Any]:
    try:
        obj = extract_json(output_text)
    except Exception:
        return {"json_valid": False, "schema_ok": False}

    required = expected_schema.get("required_fields", [])
    missing = [k for k in required if k not in obj]
    return {"json_valid": True, "schema_ok": len(missing) == 0, "missing_fields": missing}


def eval_adversarial(output_text: str, test: Dict[str, Any]) -> Dict[str, Any]:
    res = {"passed": True, "notes": []}
    lower = output_text.lower()
    for bad in test.get("must_not_contain", []) or []:
        if bad.lower() in lower:
            res["passed"] = False
            res["notes"].append(f"contains: {bad}")
    if test.get("must_be_valid_json"):
        try:
            extract_json(output_text)
        except Exception:
            res["passed"] = False
            res["notes"].append("not valid json")
    if test.get("expected_entities"):
        metrics = eval_entity_extraction(output_text, test["expected_entities"])
        if metrics["f1"] < 0.7:
            res["passed"] = False
            res["notes"].append(f"f1={metrics['f1']:.2f}")
    return res


def load_jsonl(path: Path) -> List[Dict[str, Any]]:
    items = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            items.append(json.loads(line))
    return items


def main() -> None:
    parser = argparse.ArgumentParser(description="ApexIntel evaluation harness")
    parser.add_argument("--model-dir", default=str(WORK / "models" / "base"))
    parser.add_argument("--adapter", default=str(WORK / "training" / "outputs" / "phase2_sft" / "best_adapter"))
    parser.add_argument("--eval-dir", default=str(WORK / "training_data" / "evaluation"))
    parser.add_argument("--max-new-tokens", type=int, default=1024)
    parser.add_argument("--max-examples", type=int, default=0,
                        help="Limit examples per file (0 = all). Useful for smoke tests.")
    parser.add_argument("--out-report", default=str(WORK / "training" / "outputs" / "eval_report.json"))
    parser.add_argument("--dry-run", action="store_true",
                        help="Validate eval data without loading model or running inference")
    args = parser.parse_args()

    eval_dir = Path(args.eval_dir)
    out_report = Path(args.out_report)

    if args.dry_run:
        print("═" * 60)
        print("  DRY RUN — Validating evaluation data only")
        print("═" * 60)
        return _dry_run(eval_dir, out_report)

    model, tokenizer = load_model(args.model_dir, args.adapter)

    results = {
        "model": args.model_dir,
        "adapter": args.adapter,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "total": 0,
        "passed": 0,
        "failed": 0,
        "by_type": {},
        "failures": [],
    }

    files = sorted([p for p in eval_dir.glob("*.jsonl") if p.is_file()])
    t0 = time.time()

    for path in files:
        items = load_jsonl(path)
        if args.max_examples > 0:
            items = items[:args.max_examples]

        print(f"\n── {path.name} ({len(items)} examples) ──")

        for idx, item in enumerate(items):
            eval_type = item.get("eval_type") or item.get("task") or path.stem
            results["total"] += 1
            if eval_type not in results["by_type"]:
                results["by_type"][eval_type] = {"total": 0, "passed": 0, "failed": 0}
            results["by_type"][eval_type]["total"] += 1

            system = None
            user = None
            if "input" in item and isinstance(item["input"], dict):
                system = item["input"].get("system")
                user = item["input"].get("user")
            elif "input" in item:
                user = item["input"]

            if not system:
                system = DEFAULT_SYSTEMS.get(item.get("task", "entity_extraction"), DEFAULT_SYSTEMS["entity_extraction"])
            if user is None:
                user = ""

            output_text = generate(model, tokenizer, system, user, args.max_new_tokens)

            passed = True
            metrics = {}
            if eval_type == "recipe_quality":
                metrics = eval_recipe_quality(output_text, item.get("expected_schema", {}))
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type == "entity_extraction":
                metrics = eval_entity_extraction(output_text, item.get("ground_truth", item.get("expected_entities", {})))
                passed = metrics["f1"] >= 0.7
            elif eval_type == "poi_synthesis":
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type == "memo_quality":
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type in ("adversarial_tests", "adversarial"):
                metrics = eval_adversarial(output_text, item)
                passed = metrics["passed"]
            elif eval_type in ("regression_tests", "multilingual_golden", "recipe_hypothesis"):
                metrics = eval_entity_extraction(output_text, item.get("expected_entities", item.get("expected", {})))
                passed = metrics["f1"] >= 0.7
            else:
                metrics = {"note": "no evaluator"}
                passed = False  # Unknown eval types must not silently pass

            if passed:
                results["passed"] += 1
                results["by_type"][eval_type]["passed"] += 1
            else:
                results["failed"] += 1
                results["by_type"][eval_type]["failed"] += 1
                results["failures"].append({
                    "id": item.get("id"),
                    "type": eval_type,
                    "file": path.name,
                    "metrics": {k: v for k, v in metrics.items() if isinstance(v, (int, float, bool, str))},
                })

            # Progress indicator
            if (idx + 1) % 50 == 0:
                print(f"    [{idx + 1}/{len(items)}] …")

    elapsed = time.time() - t0
    results["elapsed_seconds"] = round(elapsed, 1)
    results["pass_rate"] = round(results["passed"] / max(results["total"], 1) * 100, 1)

    out_report.parent.mkdir(parents=True, exist_ok=True)
    with open(out_report, "w", encoding="utf-8") as f:
        json.dump(results, f, indent=2)

    # Print summary table
    print("\n" + "═" * 60)
    print("  EVALUATION SUMMARY")
    print("═" * 60)
    print(f"  Total:   {results['total']}")
    print(f"  Passed:  {results['passed']}")
    print(f"  Failed:  {results['failed']}")
    print(f"  Rate:    {results['pass_rate']}%")
    print(f"  Time:    {elapsed:.0f}s")
    print("─" * 60)
    for etype, counts in sorted(results["by_type"].items()):
        rate = counts["passed"] / max(counts["total"], 1) * 100
        status = "✓" if rate >= 80 else "⚠" if rate >= 50 else "✗"
        print(f"  {status} {etype:<30} {counts['passed']}/{counts['total']} ({rate:.0f}%)")
    print("═" * 60)
    print(f"  Report: {out_report}")


def _dry_run(eval_dir: Path, out_report: Path) -> None:
    """Validate all eval JSONL files without a model."""
    files = sorted([p for p in eval_dir.glob("*.jsonl") if p.is_file()])
    total = 0
    valid = 0
    by_type: Dict[str, int] = {}

    for path in files:
        items = load_jsonl(path)
        file_valid = 0
        for item in items:
            total += 1
            eval_type = item.get("eval_type") or item.get("task") or path.stem
            by_type[eval_type] = by_type.get(eval_type, 0) + 1

            # Validate structure
            has_input = "input" in item
            has_id = "id" in item or "test_name" in item
            if has_input and has_id:
                file_valid += 1
                valid += 1

        status = "✓" if file_valid == len(items) else "⚠"
        print(f"  {status} {path.name}: {file_valid}/{len(items)} valid examples")

    print(f"\n  Total: {total} examples, {valid} valid")
    print(f"  Types: {', '.join(sorted(by_type.keys()))}")
    for etype, count in sorted(by_type.items()):
        print(f"    {etype}: {count}")

    # Write dry-run report
    report = {
        "dry_run": True,
        "total": total,
        "valid": valid,
        "by_type": by_type,
    }
    out_report.parent.mkdir(parents=True, exist_ok=True)
    with open(out_report, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)
    print(f"\n  Report: {out_report}")

    if valid < 500:
        print(f"\n  ✗ Only {valid} valid examples (target ≥ 500)")
        import sys
        sys.exit(1)
    else:
        print(f"\n  ✓ {valid} valid examples (≥ 500 threshold met)")


if __name__ == "__main__":
    main()
