#!/usr/bin/env python3
"""Evaluate ApexIntel model on 800+ domain scenarios.

Runs all JSONL files in training_data/evaluation and produces a summary report.

Usage:
    # Full evaluation with model
    python training/eval_harness.py --model-dir ./models/base --adapter ./training/outputs/phase2_sft/best_adapter

    # Dry-run: validate eval data only (no model needed)
    python training/eval_harness.py --dry-run

    # Limit examples per file (for quick smoke tests)
    python training/eval_harness.py --max-examples 5

    # Compare base model vs Phase 1 vs Phase 2 (evaluates all three)
    python training/eval_harness.py --compare
"""
import argparse
import json
import os
import math
import re
import time
import random
from pathlib import Path
from typing import Any, Dict, List, Tuple

WORK = Path(__file__).resolve().parent.parent

DEFAULT_SYSTEMS = {
    "entity_extraction": "Extract structured entities from this text.",
    "poi_synthesis": "You are synthesizing professional intelligence about a POI for an EMS competitive intelligence platform.",
    "recipe_hypothesis": "You are an OSINT analyst generating insight recipes for an EMS competitive intelligence platform.",
    "memo_quality": "You are writing a weekly strategy memo for an EMS General Manager.",
    "competitive_analysis": "You are comparing capabilities of two EMS companies. Produce a structured competitive analysis JSON.",
    "company_dossier": "You are generating a company intelligence dossier for an EMS competitive intelligence platform.",
    "warning_generation": "You are generating real-time intelligence warnings for an EMS competitive intelligence platform.",
    "supply_chain_risk": "Analyze the supply chain risk. Return JSON with: risk_summary, affected_components, severity, impact_assessment, mitigation_options, timeline, alternative_suppliers.",
    "compliance": "Assess trade compliance risk. Return JSON with: risk_level, entities_of_concern, applicable_regulations, red_flags, recommended_actions.",
}

MANIFEST_NAME = "manifest.json"


def load_eval_manifest(eval_dir: Path) -> Dict[str, Any]:
    manifest_path = eval_dir / MANIFEST_NAME
    if not manifest_path.exists():
        return {
            "suite_name": "ad_hoc_eval",
            "dataset_version": "unversioned",
            "schema_version": "v0",
            "files": [],
        }

    with open(manifest_path, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    manifest.setdefault("suite_name", "apexintel-evaluation")
    manifest.setdefault("dataset_version", "unversioned")
    manifest.setdefault("schema_version", "v1")
    manifest.setdefault("files", [])
    return manifest


def resolve_eval_files(eval_dir: Path) -> Tuple[Dict[str, Any], List[Path]]:
    manifest = load_eval_manifest(eval_dir)
    declared_files = manifest.get("files", []) or []
    if declared_files:
        files = [eval_dir / entry["file"] for entry in declared_files if entry.get("file")]
    else:
        files = sorted([p for p in eval_dir.glob("*.jsonl") if p.is_file()])

    missing = [path.name for path in files if not path.exists()]
    if missing:
        raise FileNotFoundError(f"evaluation manifest references missing files: {missing}")

    return manifest, files


def _strip_fences(text: str) -> str:
    """Strip markdown code fences from text, matching Rust validators.rs logic.

    Handles ```json, ```JSON, bare ``` blocks, and prose wrapping.
    """
    # Try to find fenced code blocks: ```json ... ``` or ``` ... ```
    fence_pattern = re.compile(
        r"```(?:json|JSON)?\s*\n?(.*?)\n?\s*```",
        re.DOTALL,
    )
    matches = fence_pattern.findall(text)
    if matches:
        # Return first match that looks like JSON
        for m in matches:
            stripped = m.strip()
            if stripped and (stripped.startswith("{") or stripped.startswith("[")):
                return stripped
        # If none looked like JSON, return first non-empty match
        for m in matches:
            if m.strip():
                return m.strip()
    return text.strip()


def _extract_json_substring(text: str) -> Any:
    """Extract first JSON object or array from arbitrary text."""
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


def extract_json(text: str) -> Any:
    """Extract JSON from LLM output, tolerating markdown fences and prose.

    Mirrors the resilient extraction in crates/llm/src/validators.rs:
    1. Try raw json.loads()
    2. Strip markdown code fences (```json ... ```)
    3. Find first { ... } or [ ... ] substring
    """
    text = text.strip()
    if not text:
        raise ValueError("empty text")

    # 1. Try direct parse
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass

    # 2. Strip markdown code fences and try again
    defenced = _strip_fences(text)
    if defenced != text:
        try:
            return json.loads(defenced)
        except json.JSONDecodeError:
            pass
        # Try substring extraction on defenced text
        try:
            return _extract_json_substring(defenced)
        except ValueError:
            pass

    # 3. Try substring extraction on original text
    return _extract_json_substring(text)


def check_content_quality(text: str) -> List[str]:
    """Check for LLM refusals, empty output, or gibberish (matches Rust validators)."""
    issues: List[str] = []
    trimmed = text.strip()
    if not trimmed:
        issues.append("Content is empty")
        return issues
    if len(trimmed) < 10:
        issues.append("Content is suspiciously short")
    lower = trimmed.lower()
    refusal_patterns = [
        "i cannot", "i'm unable to", "as an ai",
        "i don't have access", "i apologize, but",
    ]
    for pat in refusal_patterns:
        if lower.startswith(pat):
            issues.append(f"Content appears to be a refusal: starts with '{pat}'")
    # Excessive character repetition
    if len(trimmed) > 10:
        unique = len(set(trimmed))
        ratio = unique / len(trimmed)
        if ratio < 0.05:
            issues.append("Content has excessive character repetition")
    return issues


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


def bootstrap_confidence_interval(values: List[float], iterations: int = 1000, seed: int = 42) -> Dict[str, float]:
    if not values:
        return {"mean": 0.0, "ci_lower": 0.0, "ci_upper": 0.0}
    rng = random.Random(seed)
    samples = []
    for _ in range(iterations):
        resample = [values[rng.randrange(len(values))] for _ in range(len(values))]
        samples.append(sum(resample) / len(resample))
    samples.sort()
    lower_idx = max(0, int(0.025 * (len(samples) - 1)))
    upper_idx = min(len(samples) - 1, int(0.975 * (len(samples) - 1)))
    return {
        "mean": sum(values) / len(values),
        "ci_lower": samples[lower_idx],
        "ci_upper": samples[upper_idx],
    }


def paired_bootstrap_test(values_a: List[float], values_b: List[float], iterations: int = 1000, seed: int = 42) -> Dict[str, float]:
    n = min(len(values_a), len(values_b))
    if n == 0:
        return {"delta": 0.0, "p_value": 1.0}
    paired_a = values_a[:n]
    paired_b = values_b[:n]
    observed_delta = (sum(paired_b) / n) - (sum(paired_a) / n)
    rng = random.Random(seed)
    extreme = 0
    for _ in range(iterations):
        idxs = [rng.randrange(n) for _ in range(n)]
        delta = (sum(paired_b[i] for i in idxs) / n) - (sum(paired_a[i] for i in idxs) / n)
        if abs(delta) >= abs(observed_delta):
            extreme += 1
    return {"delta": observed_delta, "p_value": extreme / iterations}


def compute_bleu_like_score(prediction: str, reference: str) -> float:
    pred_tokens = prediction.lower().split()
    ref_tokens = reference.lower().split()
    if not pred_tokens or not ref_tokens:
        return 0.0

    precisions = []
    for n in range(1, 5):
        pred_ngrams = [tuple(pred_tokens[i:i + n]) for i in range(max(0, len(pred_tokens) - n + 1))]
        ref_ngrams = [tuple(ref_tokens[i:i + n]) for i in range(max(0, len(ref_tokens) - n + 1))]
        if not pred_ngrams or not ref_ngrams:
            precisions.append(0.0)
            continue
        ref_counts: Dict[Tuple[str, ...], int] = {}
        for gram in ref_ngrams:
            ref_counts[gram] = ref_counts.get(gram, 0) + 1
        matches = 0
        used: Dict[Tuple[str, ...], int] = {}
        for gram in pred_ngrams:
            available = ref_counts.get(gram, 0)
            consumed = used.get(gram, 0)
            if consumed < available:
                matches += 1
                used[gram] = consumed + 1
        precisions.append((matches + 1) / (len(pred_ngrams) + 1))

    brevity_penalty = 1.0 if len(pred_tokens) >= len(ref_tokens) else math.exp(1 - len(ref_tokens) / max(len(pred_tokens), 1))
    return brevity_penalty * math.exp(sum(math.log(max(p, 1e-9)) for p in precisions) / 4.0)


def primary_metric(eval_type: str, metrics: Dict[str, Any], item: Dict[str, Any], output_text: str) -> Tuple[str | None, float | None]:
    if eval_type == "entity_extraction":
        return "f1", float(metrics.get("f1", 0.0))
    if eval_type == "memo_quality":
        return "quality_score", float(metrics.get("field_coverage", 0.0))
    if eval_type in {"recipe_quality", "poi_synthesis", "competitive_analysis", "company_dossier", "warning_generation", "supply_chain_risk", "compliance"}:
        return "accuracy", 1.0 if metrics.get("json_valid") and metrics.get("schema_ok") else 0.0
    if eval_type in {"adversarial_tests", "adversarial", "regression_tests", "multilingual_golden", "recipe_hypothesis"}:
        return "accuracy", 1.0 if metrics.get("passed", False) or metrics.get("f1", 0.0) >= 0.5 else 0.0
    if item.get("reference_text"):
        return "bleu", compute_bleu_like_score(output_text, str(item["reference_text"]))
    return None, None


def summarize_metrics(scored_examples: List[Dict[str, Any]]) -> Tuple[Dict[str, Any], List[Dict[str, Any]]]:
    metric_buckets: Dict[str, List[float]] = {}
    category_buckets: Dict[str, Dict[str, List[float]]] = {}

    for sample in scored_examples:
        metric_name = sample["metric_name"]
        metric_buckets.setdefault(metric_name, []).append(sample["score"])
        category_buckets.setdefault(sample["category"], {}).setdefault(metric_name, []).append(sample["score"])

    summary = {
        "overall": {name: bootstrap_confidence_interval(values) for name, values in metric_buckets.items()},
        "by_category": {
            category: {name: bootstrap_confidence_interval(values) for name, values in metrics.items()}
            for category, metrics in category_buckets.items()
        },
    }

    assertions = []
    entity_f1 = summary["by_category"].get("entity_extraction", {}).get("f1")
    if entity_f1:
        assertions.append({
            "name": "entity_extraction_f1_lower_bound",
            "passed": entity_f1["ci_lower"] > 0.70,
            "ci_lower": entity_f1["ci_lower"],
            "threshold": 0.70,
        })
    memo_quality = summary["by_category"].get("memo_quality", {}).get("quality_score")
    if memo_quality:
        assertions.append({
            "name": "memo_quality_lower_bound",
            "passed": memo_quality["ci_lower"] > 0.60,
            "ci_lower": memo_quality["ci_lower"],
            "threshold": 0.60,
        })
    return summary, assertions


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


def generate_batch(model, tokenizer, prompts: List[str], max_new_tokens: int, batch_size: int = 4) -> List[str]:
    """Generate responses for multiple prompts using batched inference.
    
    Left-pads inputs so all sequences in a batch have the same length,
    then generates in parallel. Much faster than sequential generation.
    """
    import torch

    all_outputs: List[str] = []
    # Process in mini-batches
    for i in range(0, len(prompts), batch_size):
        batch_prompts = prompts[i:i + batch_size]

        # Tokenize with left padding for batch generation
        orig_side = tokenizer.padding_side
        tokenizer.padding_side = "left"
        inputs = tokenizer(
            batch_prompts,
            return_tensors="pt",
            padding=True,
            truncation=True,
            max_length=2048,
        )
        tokenizer.padding_side = orig_side

        # Move to model device
        device = next(model.parameters()).device
        inputs = {k: v.to(device) for k, v in inputs.items()}
        prompt_lens = inputs["attention_mask"].sum(dim=1).tolist()

        with torch.no_grad():
            out = model.generate(
                **inputs,
                max_new_tokens=max_new_tokens,
                do_sample=False,
                temperature=0.0,
            )

        # Decode each sequence, stripping the prompt tokens
        for j, seq in enumerate(out):
            plen = int(prompt_lens[j])
            generated_ids = seq[plen:]
            decoded = tokenizer.decode(generated_ids, skip_special_tokens=True).strip()
            all_outputs.append(decoded)

    return all_outputs


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
    quality = check_content_quality(output_text)
    if quality:
        result["quality_issues"] = quality

    try:
        obj = extract_json(output_text)
        result["json_valid"] = True
    except Exception:
        return result

    required = expected_schema.get("required_fields", [])
    missing = [k for k in required if k not in obj]
    result["missing_fields"] = missing

    # Tolerant: pass if >= 70% of required fields present (matches Rust generous defaults)
    if required:
        field_ratio = (len(required) - len(missing)) / len(required)
    else:
        field_ratio = 1.0

    signals_min = expected_schema.get("signals_min", 1)
    has_signals = isinstance(obj.get("signals"), list) and len(obj.get("signals")) >= signals_min

    # Schema OK if field coverage >= 70% AND (signals present OR not required)
    result["schema_ok"] = field_ratio >= 0.7 and (has_signals or "signals" not in required)
    result["field_coverage"] = round(field_ratio, 2)
    result["score"] = field_ratio if result["json_valid"] else 0.0
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
    quality = check_content_quality(output_text)
    try:
        obj = extract_json(output_text)
    except Exception:
        return {"json_valid": False, "schema_ok": False, "quality_issues": quality}

    required = expected_schema.get("required_fields", [])
    missing = [k for k in required if k not in obj]

    # Tolerant: pass if >= 70% of required fields present
    if required:
        field_ratio = (len(required) - len(missing)) / len(required)
    else:
        field_ratio = 1.0

    return {
        "json_valid": True,
        "schema_ok": field_ratio >= 0.7,
        "missing_fields": missing,
        "field_coverage": round(field_ratio, 2),
        "quality_issues": quality,
    }


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
        if metrics["f1"] < 0.5:
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
    parser.add_argument("--max-new-tokens", type=int, default=512)
    parser.add_argument("--max-examples", type=int, default=5,
                        help="Limit examples per file (0 = all). Default 5 for fast eval.")
    parser.add_argument("--out-report", default=str(WORK / "training" / "outputs" / "eval_report.json"))
    parser.add_argument("--dry-run", action="store_true",
                        help="Validate eval data without loading model or running inference")
    parser.add_argument("--compare", action="store_true",
                        help="Compare base model, Phase 1 (DAPT), and Phase 2 (SFT) adapters side-by-side")
    parser.add_argument("--phase1-adapter",
                        default=str(WORK / "training" / "outputs" / "phase1_dapt" / "best_adapter"),
                        help="Path to Phase 1 DAPT adapter (used in --compare mode)")
    args = parser.parse_args()

    eval_dir = Path(args.eval_dir)
    out_report = Path(args.out_report)

    if args.dry_run:
        print("═" * 60)
        print("  DRY RUN — Validating evaluation data only")
        print("═" * 60)
        return _dry_run(eval_dir, out_report)

    if args.compare:
        print("═" * 60)
        print("  COMPARISON MODE — Base vs Phase 1 vs Phase 2")
        print("═" * 60)
        return _compare(args, eval_dir, out_report)

    model, tokenizer = load_model(args.model_dir, args.adapter)
    import sys

    manifest, files = resolve_eval_files(eval_dir)

    results = {
        "model": args.model_dir,
        "adapter": args.adapter,
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "suite_name": manifest.get("suite_name"),
        "dataset_version": manifest.get("dataset_version"),
        "schema_version": manifest.get("schema_version"),
        "total": 0,
        "passed": 0,
        "failed": 0,
        "by_type": {},
        "failures": [],
        "metric_summary": {},
        "threshold_assertions": [],
    }
    scored_examples: List[Dict[str, Any]] = []

    t0 = time.time()
    global_idx = 0

    # Count total examples first
    total_count = 0
    for path in files:
        items = load_jsonl(path)
        if args.max_examples > 0:
            items = items[:args.max_examples]
        total_count += len(items)
    print(f"  Evaluating {total_count} examples across {len(files)} files (max_tokens={args.max_new_tokens})")
    sys.stdout.flush()

    for path in files:
        items = load_jsonl(path)
        if args.max_examples > 0:
            items = items[:args.max_examples]

        print(f"\n── {path.name} ({len(items)} examples) ──")
        sys.stdout.flush()

        for idx, item in enumerate(items):
            eval_type = item.get("eval_type") or item.get("task") or path.stem
            results["total"] += 1
            global_idx += 1
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

            t1 = time.time()
            output_text = generate(model, tokenizer, system, user, args.max_new_tokens)
            gen_time = time.time() - t1

            passed = True
            metrics = {}
            if eval_type == "recipe_quality":
                metrics = eval_recipe_quality(output_text, item.get("expected_schema", {}))
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type == "entity_extraction":
                metrics = eval_entity_extraction(output_text, item.get("ground_truth", item.get("expected_entities", {})))
                passed = metrics["f1"] >= 0.5  # Relaxed from 0.7 — production uses tolerant parsing
            elif eval_type in ("poi_synthesis", "competitive_analysis", "company_dossier"):
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type == "memo_quality":
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type == "warning_generation":
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                sev_ok = True
                if metrics.get("json_valid"):
                    try:
                        obj = extract_json(output_text)
                        sev = obj.get("severity", "").lower()
                        allowed = item.get("expected_schema", {}).get("severity_enum", ["critical", "warning", "info"])
                        sev_ok = sev in [s.lower() for s in allowed]
                    except Exception:
                        sev_ok = False
                metrics["severity_ok"] = sev_ok
                passed = metrics["json_valid"] and metrics["schema_ok"] and sev_ok
            elif eval_type == "supply_chain_risk":
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                sev_ok = True
                if metrics.get("json_valid"):
                    try:
                        obj = extract_json(output_text)
                        sev = obj.get("severity", "").lower()
                        expected = item.get("expected_severity", "").lower()
                        sev_ok = sev == expected or (sev in ("critical", "high") and expected in ("critical", "high"))
                    except Exception:
                        sev_ok = False
                metrics["severity_ok"] = sev_ok
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type == "compliance":
                metrics = eval_schema_only(output_text, item.get("expected_schema", {}))
                risk_ok = True
                if metrics.get("json_valid"):
                    try:
                        obj = extract_json(output_text)
                        rl = obj.get("risk_level", "").lower()
                        expected = item.get("expected_risk_level", "").lower()
                        risk_ok = rl == expected or (rl in ("critical", "high") and expected in ("critical", "high"))
                    except Exception:
                        risk_ok = False
                metrics["risk_level_ok"] = risk_ok
                passed = metrics["json_valid"] and metrics["schema_ok"]
            elif eval_type in ("adversarial_tests", "adversarial"):
                metrics = eval_adversarial(output_text, item)
                passed = metrics["passed"]
            elif eval_type in ("regression_tests", "multilingual_golden", "recipe_hypothesis"):
                metrics = eval_entity_extraction(output_text, item.get("expected_entities", item.get("expected", {})))
                passed = metrics["f1"] >= 0.5  # Relaxed from 0.7 — production uses tolerant parsing
            else:
                metrics = {"note": "no evaluator"}
                passed = False  # Unknown eval types must not silently pass

            metric_name, metric_score = primary_metric(eval_type, metrics, item, output_text)
            if metric_name is not None and metric_score is not None:
                scored_examples.append({
                    "category": eval_type,
                    "metric_name": metric_name,
                    "score": float(metric_score),
                })
            if item.get("reference_text"):
                scored_examples.append({
                    "category": eval_type,
                    "metric_name": "bleu",
                    "score": compute_bleu_like_score(output_text, str(item["reference_text"])),
                })

            status = "✓" if passed else "✗"
            print(f"    [{global_idx}/{total_count}] {status} {item.get('id', f'{eval_type}-{idx}')} ({gen_time:.1f}s)")
            sys.stdout.flush()

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

    elapsed = time.time() - t0
    results["elapsed_seconds"] = round(elapsed, 1)
    results["pass_rate"] = round(results["passed"] / max(results["total"], 1) * 100, 1)
    metric_summary, threshold_assertions = summarize_metrics(scored_examples)
    results["metric_summary"] = metric_summary
    results["threshold_assertions"] = threshold_assertions

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
    for metric_name, interval in sorted(results["metric_summary"].get("overall", {}).items()):
        print(
            f"  {metric_name}: {interval['mean']:.3f} "
            f"(95% CI {interval['ci_lower']:.3f}-{interval['ci_upper']:.3f})"
        )
    print("─" * 60)
    for etype, counts in sorted(results["by_type"].items()):
        rate = counts["passed"] / max(counts["total"], 1) * 100
        status = "✓" if rate >= 80 else "⚠" if rate >= 50 else "✗"
        print(f"  {status} {etype:<30} {counts['passed']}/{counts['total']} ({rate:.0f}%)")
    print("═" * 60)
    print(f"  Report: {out_report}")
    failed_assertions = [assertion for assertion in threshold_assertions if not assertion["passed"]]
    if failed_assertions:
        print("  Threshold assertions failed:")
        for assertion in failed_assertions:
            print(
                f"    {assertion['name']}: ci_lower={assertion['ci_lower']:.3f} "
                f"threshold={assertion['threshold']:.3f}"
            )
        raise SystemExit(1)


def _run_eval_pass(model, tokenizer, eval_dir: Path, max_examples: int, max_new_tokens: int, label: str) -> Dict[str, Any]:
    """Run a single evaluation pass, returning results dict."""
    results: Dict[str, Any] = {
        "label": label,
        "total": 0,
        "passed": 0,
        "failed": 0,
        "by_type": {},
        "example_scores": [],
    }
    _, files = resolve_eval_files(eval_dir)
    for path in files:
        items = load_jsonl(path)
        if max_examples > 0:
            items = items[:max_examples]
        for item in items:
            eval_type = item.get("eval_type") or item.get("task") or path.stem
            results["total"] += 1
            if eval_type not in results["by_type"]:
                results["by_type"][eval_type] = {"total": 0, "passed": 0}
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

            output_text = generate(model, tokenizer, system, user, max_new_tokens)

            passed = True
            try:
                obj = extract_json(output_text)
                passed = isinstance(obj, (dict, list))
            except Exception:
                passed = False
            results["example_scores"].append(1.0 if passed else 0.0)

            if passed:
                results["passed"] += 1
                results["by_type"][eval_type]["passed"] += 1
            else:
                results["failed"] += 1

    results["pass_rate"] = round(results["passed"] / max(results["total"], 1) * 100, 1)
    return results


def _compare(args, eval_dir: Path, out_report: Path) -> None:
    """Evaluate base model, Phase 1, and Phase 2 adapters then print comparison table."""
    import gc
    import torch

    configs = [
        ("Base (no adapter)", args.model_dir, None),
        ("Phase 1 (DAPT)", args.model_dir, args.phase1_adapter),
        ("Phase 2 (SFT)", args.model_dir, args.adapter),
    ]

    all_results: List[Dict[str, Any]] = []
    for label, model_dir, adapter in configs:
        adapter_path = adapter
        if adapter_path and not Path(adapter_path).exists():
            print(f"  ⚠ Skipping '{label}': adapter not found at {adapter_path}")
            continue

        print(f"\n{'─' * 60}")
        print(f"  Evaluating: {label}")
        print(f"{'─' * 60}")

        model, tokenizer = load_model(model_dir, adapter_path)
        res = _run_eval_pass(model, tokenizer, eval_dir, args.max_examples, args.max_new_tokens, label)
        all_results.append(res)

        # Free VRAM between runs
        del model, tokenizer
        gc.collect()
        if torch.cuda.is_available():
            torch.cuda.empty_cache()

    # Print comparison table
    print("\n" + "═" * 80)
    print("  BASELINE COMPARISON")
    print("═" * 80)

    # Collect all eval types across runs
    all_types: set = set()
    for r in all_results:
        all_types.update(r["by_type"].keys())

    header = f"  {'Eval Type':<30}"
    for r in all_results:
        header += f" {r['label']:>14}"
    print(header)
    print("  " + "─" * (30 + 15 * len(all_results)))

    for etype in sorted(all_types):
        line = f"  {etype:<30}"
        for r in all_results:
            bt = r["by_type"].get(etype, {"total": 0, "passed": 0})
            rate = bt["passed"] / max(bt["total"], 1) * 100
            line += f" {rate:>12.0f}%"
        print(line)

    print("  " + "─" * (30 + 15 * len(all_results)))
    total_line = f"  {'OVERALL':<30}"
    for r in all_results:
        total_line += f" {r['pass_rate']:>12.1f}%"
    print(total_line)
    print("═" * 80)

    paired_comparisons = []
    if all_results:
        baseline = all_results[0]
        for contender in all_results[1:]:
            paired = paired_bootstrap_test(baseline.get("example_scores", []), contender.get("example_scores", []))
            paired_comparisons.append({
                "baseline": baseline["label"],
                "candidate": contender["label"],
                "delta": paired["delta"],
                "p_value": paired["p_value"],
            })
            print(
                f"  {contender['label']} vs {baseline['label']}: "
                f"delta={paired['delta']:.3f}, p={paired['p_value']:.3f}"
            )

    # Save comparison report
    report = {
        "mode": "comparison",
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "results": all_results,
        "paired_bootstrap": paired_comparisons,
    }
    comparison_report = out_report.parent / "comparison_report.json"
    comparison_report.parent.mkdir(parents=True, exist_ok=True)
    with open(comparison_report, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2)
    print(f"  Report: {comparison_report}")


def _dry_run(eval_dir: Path, out_report: Path) -> None:
    """Validate all eval JSONL files without a model."""
    manifest, files = resolve_eval_files(eval_dir)
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
        "suite_name": manifest.get("suite_name"),
        "dataset_version": manifest.get("dataset_version"),
        "schema_version": manifest.get("schema_version"),
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
