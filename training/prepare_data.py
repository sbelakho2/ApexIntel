#!/usr/bin/env python3
"""
prepare_data.py — Consolidate all ApexIntel training data into ready-to-train
JSONL splits (DAPT corpus + SFT instruction data + eval sets).

Run once before training:
    python training/prepare_data.py --work-dir /workspace/ApexIntel
"""

import argparse
import glob
import json
import os
import random
import csv
import sys
from pathlib import Path

random.seed(42)

# ─── helpers ───────────────────────────────────────────────────────

def count_lines(path: str) -> int:
    with open(path, "r") as f:
        return sum(1 for _ in f)


def emit(records: list[dict], path: str):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        for r in records:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"  → {path}  ({len(records):,} records, {os.path.getsize(path)/1e6:.1f} MB)")


# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# 1.  DAPT corpus  (plain text → {"text": "..."} )
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def build_dapt_corpus(work: str) -> list[dict]:
    """Collect all domain text into a list of {"text": ...} records."""
    td = os.path.join(work, "training_data")
    records: list[dict] = []

    # ── Patent abstracts (TSV: patent_id \t date \t title) ──
    pat_file = os.path.join(td, "patents", "ems_patent_abstracts.tsv")
    if os.path.exists(pat_file):
        print("  loading patent abstracts …")
        with open(pat_file, "r") as f:
            reader = csv.reader(f, delimiter="\t")
            for row in reader:
                if len(row) >= 3 and row[2].strip():
                    records.append({"text": row[2].strip()})
        print(f"    {len(records):,} patent titles loaded")

    # ── NER training data as domain text ──
    ner_dir = os.path.join(td, "ner_models")
    for jl in glob.glob(os.path.join(ner_dir, "*.jsonl")):
        print(f"  loading NER corpus: {os.path.basename(jl)} …")
        before = len(records)
        with open(jl, "r") as f:
            for line in f:
                try:
                    obj = json.loads(line)
                    # MultiNERD format: {"tokens": [...], "ner_tags": [...]}
                    tokens = obj.get("tokens", [])
                    if tokens:
                        records.append({"text": " ".join(tokens)})
                except json.JSONDecodeError:
                    pass
        print(f"    +{len(records) - before:,} NER sentences")

    # ── Seed articles ──
    seeds_file = os.path.join(td, "corpus_seeds", "domain_seed_articles.json")
    if os.path.exists(seeds_file):
        print("  loading seed articles …")
        with open(seeds_file, "r") as f:
            articles = json.load(f)
        for a in articles:
            body = a.get("body", a.get("content", ""))
            title = a.get("title", "")
            if body:
                records.append({"text": f"{title}\n\n{body}" if title else body})

    # ── Reference JSON files (flatten to text blocks) ──
    ref_dirs = [
        "commodities", "certifications", "cybersecurity", "defense",
        "economics", "electronics", "geopolitics", "supply_chain",
        "geography", "psychology", "nlp",
    ]
    for subdir in ref_dirs:
        ref_path = os.path.join(td, subdir)
        if not os.path.isdir(ref_path):
            continue
        for jf in glob.glob(os.path.join(ref_path, "*.json")):
            try:
                with open(jf, "r") as f:
                    data = json.load(f)
                text = json.dumps(data, indent=2, ensure_ascii=False)
                # Break large JSON files into chunks of ~3000 chars
                if len(text) > 4000:
                    chunks = [text[i:i+3000] for i in range(0, len(text), 3000)]
                    for chunk in chunks:
                        records.append({"text": chunk})
                else:
                    records.append({"text": text})
            except (json.JSONDecodeError, UnicodeDecodeError):
                pass

    # ── TED tender records ──
    ted_file = os.path.join(td, "tenders", "ted_electronics_tenders.json")
    if os.path.exists(ted_file):
        print("  loading TED tenders …")
        with open(ted_file, "r") as f:
            tenders = json.load(f)
        for t in tenders:
            records.append({"text": json.dumps(t, ensure_ascii=False)})

    # ── SEC filings metadata ──
    filings_dir = os.path.join(td, "filings", "10k")
    if os.path.isdir(filings_dir):
        print("  loading SEC filings metadata …")
        for jf in glob.glob(os.path.join(filings_dir, "*_submissions.json")):
            try:
                with open(jf, "r") as f:
                    data = json.load(f)
                # Extract the recent filings section
                recent = data.get("filings", {}).get("recent", {})
                if recent:
                    text = json.dumps(recent, indent=2, ensure_ascii=False)
                    chunks = [text[i:i+3000] for i in range(0, len(text), 3000)]
                    for chunk in chunks:
                        records.append({"text": chunk})
            except (json.JSONDecodeError, UnicodeDecodeError):
                pass

    # ── Sanctions lists as text ──
    sanc_dir = os.path.join(td, "sanctions")
    for sf in glob.glob(os.path.join(sanc_dir, "*.csv")):
        print(f"  loading sanctions: {os.path.basename(sf)} …")
        try:
            with open(sf, "r", errors="replace") as f:
                reader = csv.DictReader(f)
                for row in reader:
                    name = row.get("name", row.get("Name", ""))
                    if name:
                        records.append({"text": json.dumps(dict(row), ensure_ascii=False)})
        except Exception:
            pass

    print(f"\n  DAPT total: {len(records):,} text records")
    return records


# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# 2.  SFT instruction data  (already in messages format)
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def build_sft_data(work: str) -> list[dict]:
    """Load all instruction tuning JSONL files."""
    sft_dir = os.path.join(work, "training_data", "instruction_tuning")
    records: list[dict] = []
    for jl in sorted(glob.glob(os.path.join(sft_dir, "*.jsonl"))):
        task = os.path.basename(jl).replace(".jsonl", "")
        before = len(records)
        with open(jl, "r") as f:
            for line in f:
                try:
                    obj = json.loads(line)
                    obj["task"] = task
                    records.append(obj)
                except json.JSONDecodeError:
                    pass
        print(f"  SFT {task}: {len(records) - before:,} examples")
    print(f"  SFT total: {len(records):,}")
    return records


# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# 3.  Split and write
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

def split_data(records: list[dict], eval_frac: float = 0.05) -> tuple[list, list]:
    random.shuffle(records)
    split_idx = max(1, int(len(records) * eval_frac))
    return records[split_idx:], records[:split_idx]


def main():
    parser = argparse.ArgumentParser(description="Prepare ApexIntel training data")
    parser.add_argument("--work-dir", default=".", help="Workspace root")
    args = parser.parse_args()
    work = os.path.abspath(args.work_dir)
    out_dir = os.path.join(work, "training", "data")
    os.makedirs(out_dir, exist_ok=True)

    print("━" * 60)
    print("  ApexIntel Training Data Preparation")
    print("━" * 60)

    # ── DAPT ──
    print("\n[1/2] Building DAPT corpus …")
    dapt = build_dapt_corpus(work)
    dapt_train, dapt_eval = split_data(dapt, eval_frac=0.02)
    emit(dapt_train, os.path.join(out_dir, "dapt_train.jsonl"))
    emit(dapt_eval, os.path.join(out_dir, "dapt_eval.jsonl"))

    # ── SFT ──
    print("\n[2/2] Building SFT dataset …")
    sft = build_sft_data(work)
    sft_train, sft_eval = split_data(sft, eval_frac=0.10)
    emit(sft_train, os.path.join(out_dir, "sft_train.jsonl"))
    emit(sft_eval, os.path.join(out_dir, "sft_eval.jsonl"))

    # ── Summary ──
    print("\n━" * 60)
    total_size = sum(
        os.path.getsize(os.path.join(out_dir, f))
        for f in os.listdir(out_dir)
        if f.endswith(".jsonl")
    )
    print(f"  Total: {total_size / 1e6:.1f} MB in {out_dir}/")
    print("━" * 60)


if __name__ == "__main__":
    main()
