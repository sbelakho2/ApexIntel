#!/usr/bin/env python3
"""
llm_stress_test.py — Post-training LLM quality verification.

Runs a curated set of domain-specific prompts against the trained model to verify
it has learned the core ApexIntel tasks: entity extraction, POI synthesis,
insight recipe generation, memo writing, supply chain analysis, and
multilingual understanding.

Unlike eval_harness.py (which runs 500+ automated JSONL scenarios), this script
tests a small number of hand-crafted prompts with strict quality gates. All must
pass for the model to be considered deployment-ready.

Usage:
    python training/llm_stress_test.py                          # test merged model
    python training/llm_stress_test.py --adapter ./training/outputs/phase2_sft/best_adapter
    python training/llm_stress_test.py --max-new-tokens 2048    # longer outputs
"""

import argparse
import json
import os
import sys
import time
from pathlib import Path
from typing import Any, Dict, List, Tuple

WORK = Path(__file__).resolve().parent.parent

# ═══════════════════════════════════════════════════════════════════
#  Curated stress-test prompts — one per core ApexIntel capability
# ═══════════════════════════════════════════════════════════════════

STRESS_TESTS: List[Dict[str, Any]] = [
    # ── 1. Entity Extraction ──────────────────────────────────────
    {
        "id": "ner-01",
        "category": "Entity Extraction",
        "system": "Extract structured entities from this text. Return valid JSON with keys: companies, persons, locations, capabilities, certifications.",
        "user": (
            "Flex Ltd. announced that CEO Revathi Advaithi will open a new SMT line "
            "in Guadalajara, Mexico, capable of 0201 component placement at 120k CPH. "
            "The facility holds ISO 13485 and IATF 16949 certifications. Dr. Marcus "
            "Yam, VP of Engineering, confirmed partnership with Jabil Inc. for joint "
            "NPI services targeting automotive radar modules."
        ),
        "checks": {
            "json_valid": True,
            "required_entities": {
                "companies": ["Flex", "Jabil"],
                "persons": ["Revathi Advaithi", "Marcus Yam"],
                "locations": ["Guadalajara", "Mexico"],
                "certifications": ["ISO 13485", "IATF 16949"],
            },
            "min_entity_count": 6,
        },
    },
    # ── 2. POI Synthesis ──────────────────────────────────────────
    {
        "id": "poi-01",
        "category": "POI Synthesis",
        "system": (
            "You are synthesizing professional intelligence about a Person of Interest "
            "for an EMS competitive intelligence platform. Return valid JSON with keys: "
            "name, title, company, focus_areas, career_signals, risk_indicators."
        ),
        "user": (
            "Compile a POI profile for Dr. Wei Chen, SVP of Advanced Packaging at "
            "ASE Technology Holding. He recently filed 3 patents on fan-out wafer-level "
            "packaging (FOWLP)  and spoke at SEMICON Taiwan 2024 about chiplet integration. "
            "He previously led R&D at TSMC's InFO group from 2015-2021. Industry sources "
            "suggest ASE is expanding capacity in Kaohsiung for AI accelerator packaging."
        ),
        "checks": {
            "json_valid": True,
            "required_fields": ["name", "title", "company", "focus_areas", "career_signals"],
            "must_contain_any": ["FOWLP", "fan-out", "chiplet", "advanced packaging"],
        },
    },
    # ── 3. Insight Recipe Generation ──────────────────────────────
    {
        "id": "recipe-01",
        "category": "Insight Recipe",
        "system": (
            "You are an OSINT analyst generating intelligence recipes for an EMS "
            "competitive intelligence platform. Return valid JSON with keys: "
            "recipe_name, hypothesis, signals, data_sources, confidence_level, actions."
        ),
        "user": (
            "Generate an insight recipe to detect whether Foxconn is shifting iPhone "
            "production capacity from Zhengzhou, China to Karnataka, India, based on "
            "publicly available signals like job postings, import/export data, "
            "construction permits, and executive travel patterns."
        ),
        "checks": {
            "json_valid": True,
            "required_fields": ["recipe_name", "hypothesis", "signals"],
            "min_signals": 3,
            "must_contain_any": ["Foxconn", "India", "China", "production"],
        },
    },
    # ── 4. Strategy Memo ──────────────────────────────────────────
    {
        "id": "memo-01",
        "category": "Strategy Memo",
        "system": (
            "You are writing a weekly strategy memo for an EMS General Manager. "
            "Return valid JSON with keys: title, executive_summary, key_developments, "
            "risk_assessment, recommendations, outlook."
        ),
        "user": (
            "Write a strategy memo addressing: (1) TSMC raising wafer prices 8% for "
            "2025 affecting our BOM costs, (2) new EU CBAM tariffs on imported PCBAs "
            "starting January 2026, (3) a key competitor Celestica winning a $200M "
            "defense electronics contract, and (4) our Penang facility reaching 95% "
            "utilization which limits new program intake."
        ),
        "checks": {
            "json_valid": True,
            "required_fields": ["title", "executive_summary", "key_developments", "recommendations"],
            "must_contain_any": ["TSMC", "CBAM", "Celestica", "Penang"],
            "min_length": 500,
        },
    },
    # ── 5. Supply Chain Risk Analysis ─────────────────────────────
    {
        "id": "supply-01",
        "category": "Supply Chain Analysis",
        "system": (
            "Analyze the supply chain risk described below. Return valid JSON with keys: "
            "risk_summary, affected_components, severity (critical/high/medium/low), "
            "mitigation_options, timeline, alternative_suppliers."
        ),
        "user": (
            "Nexperia's Hamburg fab (formerly Vishay) experienced a fire in their "
            "epitaxial growth clean room. This fab produces 40% of the global supply "
            "of automotive-grade small-signal MOSFETs (BSS138, 2N7002 families). Lead "
            "times were already at 26 weeks. Our Q2 production of 3 automotive ECU "
            "programs depends on 500K units/month from this fab."
        ),
        "checks": {
            "json_valid": True,
            "required_fields": ["risk_summary", "severity", "mitigation_options"],
            "must_contain_any": ["Nexperia", "MOSFET", "automotive"],
            "severity_in": ["critical", "high"],
        },
    },
    # ── 6. Sanctions/Compliance Check ─────────────────────────────
    {
        "id": "compliance-01",
        "category": "Compliance",
        "system": (
            "Assess the trade compliance risk for the described scenario. Return valid "
            "JSON with keys: risk_level, entities_of_concern, applicable_regulations, "
            "red_flags, recommended_actions."
        ),
        "user": (
            "Our procurement team received an unusually low quote from Huawei Marine "
            "(now HMN Technologies) for undersea fiber optic repeaters destined for a "
            "telecom project in the UAE via a Singapore intermediary. The end customer "
            "is listed as 'Gulf Digital Infrastructure LLC' which was incorporated 6 "
            "months ago with no prior trade history."
        ),
        "checks": {
            "json_valid": True,
            "required_fields": ["risk_level", "red_flags", "recommended_actions"],
            "must_contain_any": ["Huawei", "HMN", "Entity List", "sanctions", "due diligence"],
            "risk_level_in": ["critical", "high"],
        },
    },
    # ── 7. Multilingual Entity Extraction ─────────────────────────
    {
        "id": "multi-01",
        "category": "Multilingual",
        "system": "Extract structured entities from this text. Return valid JSON with keys: companies, persons, locations, capabilities.",
        "user": (
            "村田製作所は京都本社で、代表取締役社長の中島規巨氏が、MLCCの新生産ラインへの"
            "1,200億円の設備投資を発表しました。タイのラムチャバン工場でも生産能力を30%増強します。"
            "TDKとの競争が激化する中、自動車向け高信頼性コンデンサの需要が急増しています。"
        ),
        "checks": {
            "json_valid": True,
            "required_entities": {
                "companies": ["村田製作所", "Murata", "TDK"],
                "persons": ["中島規巨", "Nakajima"],
                "locations": ["京都", "Kyoto", "ラムチャバン", "Laem Chabang", "タイ", "Thailand"],
            },
            "min_entity_count": 4,
        },
    },
    # ── 8. Multi-step Reasoning ───────────────────────────────────
    {
        "id": "reasoning-01",
        "category": "Reasoning",
        "system": (
            "You are an EMS industry analyst. Analyze the given data and provide structured "
            "reasoning. Return valid JSON with keys: analysis, conclusion, confidence, "
            "supporting_evidence, counter_arguments."
        ),
        "user": (
            "Three signals detected this week:\n"
            "1. Benchmark Electronics posted 47 new job listings for RF engineers in Tempe, AZ\n"
            "2. A $180M DoD contract for Next-Gen Jammer (NGJ) pods was awarded but the prime contractor is classified\n"
            "3. Benchmark's CEO mentioned 'significant defense program wins' on last quarter's earnings call\n\n"
            "What can we infer about Benchmark's involvement in the NGJ program?"
        ),
        "checks": {
            "json_valid": True,
            "required_fields": ["analysis", "conclusion", "confidence", "supporting_evidence"],
            "must_contain_any": ["Benchmark", "NGJ", "defense", "RF"],
            "min_length": 300,
        },
    },
]


# ═══════════════════════════════════════════════════════════════════
#  Evaluation helpers
# ═══════════════════════════════════════════════════════════════════

def extract_json(text: str) -> Any:
    """Extract JSON object/array from model output."""
    text = text.strip()
    if not text:
        raise ValueError("empty response")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        pass
    # Find first { ... } or [ ... ]
    for open_ch, close_ch in [("{", "}"), ("[", "]")]:
        start = text.find(open_ch)
        end = text.rfind(close_ch)
        if start != -1 and end > start:
            try:
                return json.loads(text[start : end + 1])
            except json.JSONDecodeError:
                continue
    raise ValueError("no valid JSON found in response")


def normalize(s: str) -> str:
    return s.lower().strip()


def count_entities(obj: dict) -> int:
    count = 0
    for key in ("companies", "persons", "locations", "capabilities", "certifications"):
        val = obj.get(key)
        if isinstance(val, list):
            count += len(val)
    return count


def check_test(test: Dict[str, Any], output_text: str) -> Dict[str, Any]:
    """Run all checks for a single stress test. Returns pass/fail + details."""
    checks = test["checks"]
    result: Dict[str, Any] = {
        "id": test["id"],
        "category": test["category"],
        "passed": True,
        "failures": [],
        "output_length": len(output_text),
    }

    # 1. JSON validity
    obj = None
    if checks.get("json_valid"):
        try:
            obj = extract_json(output_text)
            result["json_valid"] = True
        except Exception as e:
            result["json_valid"] = False
            result["passed"] = False
            result["failures"].append(f"JSON parse failed: {e}")
            return result  # Can't check further without JSON

    # 2. Required fields
    if obj and "required_fields" in checks:
        missing = [f for f in checks["required_fields"] if f not in obj]
        if missing:
            result["passed"] = False
            result["failures"].append(f"Missing fields: {missing}")

    # 3. Required entities (fuzzy match — any substring match counts)
    if obj and "required_entities" in checks:
        for entity_type, expected_names in checks["required_entities"].items():
            found_any = False
            actual = obj.get(entity_type, [])
            actual_str = json.dumps(actual, ensure_ascii=False).lower()
            for name in expected_names:
                if normalize(name) in actual_str:
                    found_any = True
                    break
            if not found_any:
                result["passed"] = False
                result["failures"].append(
                    f"Entity type '{entity_type}': none of {expected_names} found in {actual}"
                )

    # 4. Minimum entity count
    if obj and "min_entity_count" in checks:
        count = count_entities(obj)
        if count < checks["min_entity_count"]:
            result["passed"] = False
            result["failures"].append(
                f"Entity count {count} < minimum {checks['min_entity_count']}"
            )

    # 5. Must contain any keyword (in full output)
    if "must_contain_any" in checks:
        output_lower = output_text.lower()
        if not any(kw.lower() in output_lower for kw in checks["must_contain_any"]):
            result["passed"] = False
            result["failures"].append(
                f"Output missing all keywords: {checks['must_contain_any']}"
            )

    # 6. Minimum signals count
    if obj and "min_signals" in checks:
        signals = obj.get("signals", [])
        if not isinstance(signals, list) or len(signals) < checks["min_signals"]:
            result["passed"] = False
            result["failures"].append(
                f"Signals count {len(signals) if isinstance(signals, list) else 0} < {checks['min_signals']}"
            )

    # 7. Severity/risk level check
    if obj and "severity_in" in checks:
        sev = normalize(str(obj.get("severity", "")))
        if sev not in [s.lower() for s in checks["severity_in"]]:
            result["passed"] = False
            result["failures"].append(f"Severity '{sev}' not in {checks['severity_in']}")

    if obj and "risk_level_in" in checks:
        rl = normalize(str(obj.get("risk_level", "")))
        if rl not in [s.lower() for s in checks["risk_level_in"]]:
            result["passed"] = False
            result["failures"].append(f"Risk level '{rl}' not in {checks['risk_level_in']}")

    # 8. Minimum response length
    if "min_length" in checks:
        if len(output_text) < checks["min_length"]:
            result["passed"] = False
            result["failures"].append(
                f"Output length {len(output_text)} < minimum {checks['min_length']}"
            )

    return result


# ═══════════════════════════════════════════════════════════════════
#  Model loading & inference
# ═══════════════════════════════════════════════════════════════════

def load_model(model_dir: str, adapter_path: str | None = None):
    import torch
    from transformers import AutoModelForCausalLM, AutoTokenizer
    from peft import PeftModel

    print(f"Loading model: {model_dir}")
    tokenizer = AutoTokenizer.from_pretrained(model_dir, trust_remote_code=True)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    model = AutoModelForCausalLM.from_pretrained(
        model_dir,
        torch_dtype=torch.bfloat16,
        device_map="cuda:0",
        attn_implementation="flash_attention_2",
        trust_remote_code=True,
    )

    if adapter_path and os.path.isdir(adapter_path):
        print(f"Merging adapter: {adapter_path}")
        model = PeftModel.from_pretrained(model, adapter_path)
        model = model.merge_and_unload()

    model.eval()
    return model, tokenizer


def generate(model, tokenizer, system: str, user: str, max_new_tokens: int) -> str:
    import torch
    messages = [
        {"role": "system", "content": system},
        {"role": "user", "content": user},
    ]
    prompt = tokenizer.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
    inputs = tokenizer(prompt, return_tensors="pt").to(model.device)
    with torch.no_grad():
        out = model.generate(
            **inputs,
            max_new_tokens=max_new_tokens,
            do_sample=False,
            temperature=1.0,    # greedy (do_sample=False overrides)
            repetition_penalty=1.05,
        )
    prompt_len = inputs["input_ids"].shape[1]
    decoded = tokenizer.decode(out[0][prompt_len:], skip_special_tokens=True)
    return decoded.strip()


# ═══════════════════════════════════════════════════════════════════
#  Main
# ═══════════════════════════════════════════════════════════════════

def main():
    parser = argparse.ArgumentParser(description="ApexIntel LLM stress test — post-training quality verification")
    parser.add_argument("--model-dir", default=str(WORK / "models" / "base"))
    parser.add_argument(
        "--adapter",
        default=str(WORK / "training" / "outputs" / "phase2_sft" / "best_adapter"),
        help="Path to LoRA adapter (or merged model dir)",
    )
    parser.add_argument("--max-new-tokens", type=int, default=1536)
    parser.add_argument(
        "--out-report",
        default=str(WORK / "training" / "outputs" / "stress_test_report.json"),
    )
    parser.add_argument(
        "--threshold", type=float, default=0.75,
        help="Minimum pass rate (0-1) to consider the model ready (default: 0.75 = 6/8)",
    )
    args = parser.parse_args()

    print("═" * 60)
    print("  ApexIntel LLM Stress Test")
    print(f"  {len(STRESS_TESTS)} curated domain tests")
    print("═" * 60)

    model, tokenizer = load_model(args.model_dir, args.adapter)

    results: List[Dict[str, Any]] = []
    passed_count = 0
    t0 = time.time()

    for i, test in enumerate(STRESS_TESTS):
        print(f"\n── [{i+1}/{len(STRESS_TESTS)}] {test['category']}: {test['id']} ──")
        sys.stdout.flush()

        gen_t0 = time.time()
        output_text = generate(model, tokenizer, test["system"], test["user"], args.max_new_tokens)
        gen_elapsed = time.time() - gen_t0

        result = check_test(test, output_text)
        result["generation_time_s"] = round(gen_elapsed, 1)
        results.append(result)

        status = "✓ PASS" if result["passed"] else "✗ FAIL"
        print(f"  {status}  ({gen_elapsed:.1f}s, {result['output_length']} chars)")
        if not result["passed"]:
            for f in result["failures"]:
                print(f"    → {f}")
        else:
            passed_count += 1
        sys.stdout.flush()

    elapsed = time.time() - t0
    pass_rate = passed_count / len(STRESS_TESTS)

    # Write report
    report = {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S"),
        "model": args.model_dir,
        "adapter": args.adapter,
        "total_tests": len(STRESS_TESTS),
        "passed": passed_count,
        "failed": len(STRESS_TESTS) - passed_count,
        "pass_rate": round(pass_rate * 100, 1),
        "threshold": args.threshold,
        "ready": pass_rate >= args.threshold,
        "elapsed_seconds": round(elapsed, 1),
        "results": results,
    }
    out_path = Path(args.out_report)
    out_path.parent.mkdir(parents=True, exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, ensure_ascii=False)

    # Summary
    print("\n" + "═" * 60)
    print("  LLM STRESS TEST RESULTS")
    print("═" * 60)
    by_cat: Dict[str, Tuple[int, int]] = {}
    for r in results:
        cat = r["category"]
        p, t = by_cat.get(cat, (0, 0))
        by_cat[cat] = (p + (1 if r["passed"] else 0), t + 1)

    for cat, (p, t) in sorted(by_cat.items()):
        icon = "✓" if p == t else "✗"
        print(f"  {icon} {cat:<30} {p}/{t}")

    print("─" * 60)
    print(f"  Total:     {passed_count}/{len(STRESS_TESTS)} ({pass_rate*100:.0f}%)")
    print(f"  Threshold: {args.threshold*100:.0f}%")
    print(f"  Verdict:   {'✓ READY' if pass_rate >= args.threshold else '✗ NOT READY'}")
    print(f"  Time:      {elapsed:.0f}s")
    print(f"  Report:    {out_path}")
    print("═" * 60)

    sys.exit(0 if pass_rate >= args.threshold else 1)


if __name__ == "__main__":
    main()
