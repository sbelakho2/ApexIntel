import json, os, re, torch, gc
os.environ["TRANSFORMERS_VERBOSITY"] = "error"
from transformers import AutoModelForCausalLM, AutoTokenizer
from peft import PeftModel

model_dir = "/workspace/ApexIntel/training/outputs/merged_phase1"
adapter = "/workspace/ApexIntel/training/outputs/phase2_sft/best_adapter_v3"

print("Loading model...", flush=True)
tokenizer = AutoTokenizer.from_pretrained(model_dir, trust_remote_code=True)
if tokenizer.pad_token is None:
    tokenizer.pad_token = tokenizer.eos_token
model = AutoModelForCausalLM.from_pretrained(model_dir, torch_dtype=torch.bfloat16, device_map="auto", trust_remote_code=True)
model = PeftModel.from_pretrained(model, adapter)
model = model.merge_and_unload()
print("Model loaded.", flush=True)

def generate_text(model, tokenizer, system, user, max_tokens=2048):
    messages = [{"role": "system", "content": system}, {"role": "user", "content": user}]
    prompt = tokenizer.apply_chat_template(messages, tokenize=False, add_generation_prompt=True)
    inputs = tokenizer(prompt, return_tensors="pt").to(model.device)
    with torch.no_grad():
        out = model.generate(**inputs, max_new_tokens=max_tokens, do_sample=False, temperature=0.0)
    decoded = tokenizer.decode(out[0][inputs["input_ids"].shape[1]:], skip_special_tokens=True).strip()
    del inputs
    gc.collect()
    torch.cuda.empty_cache()
    return decoded

def extract_json(text):
    text = re.sub(r"<think>.*?</think>", "", text, flags=re.DOTALL).strip()
    start = text.find("{")
    end = text.rfind("}")
    if start >= 0 and end > start:
        try:
            return json.loads(text[start:end+1])
        except:
            return None
    return None

# 1. company_dossier
print("\n" + "="*60)
print("COMPANY_DOSSIER DIAGNOSIS")
print("="*60)
with open("/workspace/ApexIntel/training_data/evaluation/company_dossier_eval.jsonl") as f:
    item = json.loads(f.readline())
expected_fields = item.get("expected_output", {}).get("required_fields", [])
print(f"Expected fields: {expected_fields}")
out = generate_text(model, tokenizer, item["input"]["system"], item["input"]["user"])
obj = extract_json(out)
if obj:
    print(f"Model output keys: {sorted(obj.keys())}")
    for k in expected_fields:
        print(f"  {k}: {'PRESENT' if k in obj else 'MISSING'}")
    # Show what keys the model uses that might be aliases
    missing = [k for k in expected_fields if k not in obj]
    extra = [k for k in obj.keys() if k not in expected_fields]
    if missing and extra:
        print(f"  MISSING: {missing}")
        print(f"  EXTRA (possible aliases): {extra}")
else:
    print(f"Failed to parse JSON. Raw output (first 500):\n{out[:500]}")

# 2. entity_extraction
print("\n" + "="*60)
print("ENTITY_EXTRACTION DIAGNOSIS")
print("="*60)
with open("/workspace/ApexIntel/training_data/evaluation/entity_extraction_eval.jsonl") as f:
    item2 = json.loads(f.readline())
gt = item2.get("expected_output", {}).get("ground_truth", {})
print(f"Ground truth keys: {sorted(gt.keys())}")
for k, v in gt.items():
    if isinstance(v, list):
        print(f"  {k}: list of {len(v)} items, first={v[0] if v else 'empty'}")
    else:
        print(f"  {k}: {type(v).__name__}")
out2 = generate_text(model, tokenizer, item2["input"]["system"], item2["input"]["user"])
obj2 = extract_json(out2)
if obj2:
    print(f"Model output keys: {sorted(obj2.keys())}")
    for k in sorted(obj2.keys()):
        v = obj2[k]
        if isinstance(v, list):
            print(f"  {k}: list of {len(v)} items, first={v[0] if v else 'empty'}")
        else:
            print(f"  {k}: {type(v).__name__} = {str(v)[:100]}")
else:
    print(f"Failed to parse JSON. Raw output (first 500):\n{out2[:500]}")

# 3. warning_generation - test all 5
print("\n" + "="*60)
print("WARNING_GENERATION DIAGNOSIS")
print("="*60)
with open("/workspace/ApexIntel/training_data/evaluation/warning_generation_eval.jsonl") as f:
    lines = [json.loads(l) for l in f.readlines()]
for idx, item3 in enumerate(lines[:5]):
    sev_enum = item3.get("expected_output", {}).get("severity_enum", [])
    if not sev_enum:
        continue
    print(f"\nExample {idx}: severity_enum={sev_enum}")
    out3 = generate_text(model, tokenizer, item3["input"]["system"], item3["input"]["user"])
    obj3 = extract_json(out3)
    if obj3:
        # Find severity in top-level or nested warnings
        found_severity = False
        for k in obj3:
            if "sever" in k.lower():
                print(f"  Top-level severity '{k}': {obj3[k]}")
                found_severity = True
        if "warnings" in obj3 and isinstance(obj3["warnings"], list):
            for wi, w in enumerate(obj3["warnings"][:3]):
                if isinstance(w, dict):
                    for k in w:
                        if "sever" in k.lower():
                            print(f"  warnings[{wi}].{k}: {w[k]}")
                            found_severity = True
        if not found_severity:
            print(f"  No severity field found. Keys: {sorted(obj3.keys())}")
            print(f"  Full output (first 300): {json.dumps(obj3)[:300]}")
    else:
        print(f"  Failed to parse JSON. Raw (first 300): {out3[:300]}")

# 4. memo_quality
print("\n" + "="*60)
print("MEMO_QUALITY DIAGNOSIS")
print("="*60)
with open("/workspace/ApexIntel/training_data/evaluation/memo_quality_eval.jsonl") as f:
    item4 = json.loads(f.readline())
expected = item4.get("expected_output", {})
print(f"Expected output keys: {sorted(expected.keys())}")
req = expected.get("required_fields", expected.get("required_sections", []))
print(f"Required fields/sections: {req}")
out4 = generate_text(model, tokenizer, item4["input"]["system"], item4["input"]["user"])
obj4 = extract_json(out4)
if obj4:
    print(f"Model output keys: {sorted(obj4.keys())}")
    if req:
        for k in req:
            print(f"  {k}: {'PRESENT' if k in obj4 else 'MISSING'}")
else:
    print(f"Failed to parse JSON. Raw output (first 500):\n{out4[:500]}")

# 5. multilingual_golden
print("\n" + "="*60)
print("MULTILINGUAL_GOLDEN DIAGNOSIS")
print("="*60)
with open("/workspace/ApexIntel/training_data/evaluation/multilingual_golden.jsonl") as f:
    item5 = json.loads(f.readline())
gt5 = item5.get("expected_output", {}).get("ground_truth", {})
print(f"Ground truth keys: {sorted(gt5.keys())}")
for k, v in gt5.items():
    if isinstance(v, list):
        print(f"  {k}: list of {len(v)} items, first={v[0] if v else 'empty'}")
out5 = generate_text(model, tokenizer, item5["input"]["system"], item5["input"]["user"])
obj5 = extract_json(out5)
if obj5:
    print(f"Model output keys: {sorted(obj5.keys())}")
    for k in sorted(obj5.keys()):
        v = obj5[k]
        if isinstance(v, list):
            print(f"  {k}: list of {len(v)} items, first={v[0] if v else 'empty'}")
        else:
            print(f"  {k}: {type(v).__name__} = {str(v)[:100]}")
else:
    print(f"Failed to parse JSON. Raw output (first 500):\n{out5[:500]}")

# 6. adversarial
print("\n" + "="*60)
print("ADVERSARIAL DIAGNOSIS")
print("="*60)
with open("/workspace/ApexIntel/training_data/evaluation/adversarial_tests.jsonl") as f:
    adv_lines = [json.loads(l) for l in f.readlines()]
for idx, item6 in enumerate(adv_lines[:5]):
    must_not = item6.get("expected_output", {}).get("must_not_contain", [])
    print(f"\nExample {idx}: must_not_contain={must_not}")
    out6 = generate_text(model, tokenizer, item6["input"]["system"], item6["input"]["user"])
    clean = re.sub(r"<think>.*?</think>", "", out6, flags=re.DOTALL).strip()
    print(f"  Output (first 200): {clean[:200]}")
    for phrase in must_not:
        if phrase.lower() in clean.lower():
            print(f"  VIOLATION: contains '{phrase}'")

print("\n=== DIAGNOSIS COMPLETE ===")
del model
torch.cuda.empty_cache()
