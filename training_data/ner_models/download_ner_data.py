#!/usr/bin/env python3
"""Download NER training datasets from HuggingFace."""
import os
import shutil
from huggingface_hub import list_repo_tree, hf_hub_download

DEST = os.path.dirname(os.path.abspath(__file__))


def dl(repo_id, name, path_prefix=None, max_files=3, max_bytes=200_000_000):
    """Download data files from a HF dataset repo using recursive tree listing."""
    print(f"\nDownloading {name} from {repo_id}...")
    try:
        all_files = list(list_repo_tree(repo_id, repo_type="dataset", recursive=True))
        exts = (".parquet", ".jsonl", ".json", ".tsv", ".txt", ".conll", ".arrow")
        data_files = [
            f for f in all_files
            if hasattr(f, "path") and any(f.path.endswith(e) for e in exts)
            and not f.path.startswith(".")
        ]
        if path_prefix:
            pfx = path_prefix if isinstance(path_prefix, list) else [path_prefix]
            data_files = [f for f in data_files if any(p in f.path for p in pfx)]
        # Prefer train splits
        train_files = [f for f in data_files if "train" in f.path.lower()]
        if train_files:
            data_files = train_files
        if not data_files:
            all_paths = [f.path for f in all_files if hasattr(f, "path")][:20]
            print(f"  No data files found. Sample paths: {all_paths}")
            return
        downloaded = 0
        total_bytes = 0
        for df in data_files[:max_files]:
            safe_name = name + "_" + df.path.replace("/", "_")
            dest_path = os.path.join(DEST, safe_name)
            if os.path.exists(dest_path):
                sz = os.path.getsize(dest_path)
                print(f"  Already exists: {safe_name} ({sz:,}b)")
                downloaded += 1
                total_bytes += sz
                continue
            fs = getattr(df, "size", 0) or 0
            if total_bytes + fs > max_bytes:
                print(f"  Skip {df.path} (would exceed {max_bytes:,}b limit)")
                continue
            print(f"  {df.path} ({fs:,}b)...")
            cached = hf_hub_download(repo_id=repo_id, filename=df.path, repo_type="dataset")
            shutil.copy2(cached, dest_path)
            sz = os.path.getsize(dest_path)
            total_bytes += sz
            downloaded += 1
            print(f"  -> {safe_name} ({sz:,}b)")
        print(f"  Total: {downloaded} files, {total_bytes:,}b")
    except Exception as e:
        print(f"  Error: {e}")


def main():
    # MultiNERD - multilingual NER (EN, FR, DE, ES, IT, etc.)
    dl("Babelscape/multinerd", "multinerd", path_prefix="train", max_files=2)

    # Few-NERD - fine-grained NER (66 entity types)
    dl("DFKI-SLT/few-nerd", "fewnerd", path_prefix="supervised", max_files=2)

    # OntoNotes5 NER (18 entity types)
    dl("tner/ontonotes5", "ontonotes5", path_prefix="train", max_files=2)

    # WikiANN for multilingual NER (target languages for ApexIntel)
    for lang in ["ar", "he", "fr", "zh", "ja", "ko", "en", "de"]:
        dl(
            "unimelb-nlp/wikiann",
            f"wikiann_{lang}",
            path_prefix=f"{lang}/",
            max_files=1,
            max_bytes=50_000_000,
        )

    # NuNER - universal NER (broad coverage)
    dl("numind/NuNER", "nuner", path_prefix="train", max_files=2, max_bytes=100_000_000)

    print("\n" + "=" * 60)
    print("Done! Files:")
    skip = {"README.md", "download_ner_data.py", ".DS_Store"}
    total = 0
    for f in sorted(os.listdir(DEST)):
        fp = os.path.join(DEST, f)
        if os.path.isfile(fp) and f not in skip:
            sz = os.path.getsize(fp)
            total += sz
            print(f"  {f}: {sz:,}b")
    print(f"  TOTAL: {total:,}b")


if __name__ == "__main__":
    main()
