#!/usr/bin/env python3
"""
export_model.py — Merge LoRA adapters and export to various formats.

Supports:
  1. Merge Phase 1 + Phase 2 adapters into full-weight safetensors
  2. Export merged model to GGUF (Q4_K_M, Q5_K_M, Q8_0, F16)
  3. Push merged model / GGUF to HuggingFace Hub

Usage:
    # Merge only
    python training/export_model.py --merge

    # Merge + GGUF Q4_K_M
    python training/export_model.py --merge --gguf Q4_K_M

    # Push to HF Hub
    python training/export_model.py --merge --push --repo apexintel/model-v1
"""

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer
from peft import PeftModel

WORK = Path(__file__).resolve().parent.parent

GGUF_QUANT_TYPES = ["Q4_K_M", "Q5_K_M", "Q8_0", "F16"]


def merge_adapters(
    base_model_path: str,
    phase1_adapter: str | None,
    phase2_adapter: str | None,
    output_dir: str,
    torch_dtype: torch.dtype = torch.bfloat16,
) -> None:
    """Load base model, merge Phase 1 adapter, then Phase 2 adapter, save."""
    print(f"Loading base model: {base_model_path}")
    tokenizer = AutoTokenizer.from_pretrained(base_model_path, trust_remote_code=True)
    if tokenizer.pad_token is None:
        tokenizer.pad_token = tokenizer.eos_token

    model = AutoModelForCausalLM.from_pretrained(
        base_model_path,
        torch_dtype=torch_dtype,
        device_map="cpu",  # merge on CPU to avoid OOM
        trust_remote_code=True,
        low_cpu_mem_usage=True,
    )

    # Phase 1 merge
    if phase1_adapter and os.path.isdir(phase1_adapter):
        print(f"Merging Phase 1 adapter: {phase1_adapter}")
        model = PeftModel.from_pretrained(model, phase1_adapter)
        model = model.merge_and_unload()
        print("  ✓ Phase 1 merged")
    else:
        print(f"  ⚠ Phase 1 adapter not found at {phase1_adapter}, skipping")

    # Phase 2 merge
    if phase2_adapter and os.path.isdir(phase2_adapter):
        print(f"Merging Phase 2 adapter: {phase2_adapter}")
        model = PeftModel.from_pretrained(model, phase2_adapter)
        model = model.merge_and_unload()
        print("  ✓ Phase 2 merged")
    else:
        print(f"  ⚠ Phase 2 adapter not found at {phase2_adapter}, skipping")

    # Save merged model
    os.makedirs(output_dir, exist_ok=True)
    print(f"Saving merged model to {output_dir} …")
    model.save_pretrained(output_dir, safe_serialization=True)
    tokenizer.save_pretrained(output_dir)

    # Save merge metadata
    meta = {
        "base_model": base_model_path,
        "phase1_adapter": phase1_adapter,
        "phase2_adapter": phase2_adapter,
        "torch_dtype": str(torch_dtype),
        "merge_method": "sequential_merge_and_unload",
    }
    with open(os.path.join(output_dir, "merge_metadata.json"), "w") as f:
        json.dump(meta, f, indent=2)

    total_size = sum(
        os.path.getsize(os.path.join(output_dir, f))
        for f in os.listdir(output_dir)
        if f.endswith((".safetensors", ".bin"))
    )
    print(f"  ✓ Merged model saved ({total_size / 1e9:.1f} GB)")


def export_gguf(
    merged_dir: str,
    output_dir: str,
    quant_type: str = "Q4_K_M",
) -> str:
    """Convert merged HF model to GGUF format using llama.cpp."""
    os.makedirs(output_dir, exist_ok=True)
    gguf_file = os.path.join(output_dir, f"apexintel-{quant_type.lower()}.gguf")

    # Check if llama-cpp-python's convert script is available
    # First try the bundled convert script
    convert_script = None
    for candidate in [
        shutil.which("convert-hf-to-gguf"),
        shutil.which("convert_hf_to_gguf.py"),
    ]:
        if candidate:
            convert_script = candidate
            break

    if convert_script is None:
        # Try to find it in the Python site-packages
        try:
            import llama_cpp
            pkg_dir = Path(llama_cpp.__file__).parent
            for name in ["convert-hf-to-gguf", "convert_hf_to_gguf.py", "convert.py"]:
                candidate = pkg_dir / name
                if candidate.exists():
                    convert_script = str(candidate)
                    break
        except ImportError:
            pass

    if convert_script is None:
        # Fall back to the gguf library approach
        print("Using gguf library for conversion (llama.cpp convert script not found)")
        _export_gguf_via_library(merged_dir, gguf_file, quant_type)
    else:
        print(f"Converting to GGUF via: {convert_script}")
        # Step 1: Convert to F16 GGUF
        f16_file = os.path.join(output_dir, "apexintel-f16.gguf")
        cmd = [
            sys.executable, convert_script,
            merged_dir,
            "--outfile", f16_file,
            "--outtype", "f16",
        ]
        print(f"  Running: {' '.join(cmd)}")
        subprocess.run(cmd, check=True)

        if quant_type == "F16":
            gguf_file = f16_file
        else:
            # Step 2: Quantize
            quantize_bin = shutil.which("llama-quantize") or shutil.which("quantize")
            if quantize_bin:
                cmd = [quantize_bin, f16_file, gguf_file, quant_type]
                print(f"  Running: {' '.join(cmd)}")
                subprocess.run(cmd, check=True)
                # Clean up F16 intermediate
                if os.path.isfile(gguf_file) and gguf_file != f16_file:
                    os.remove(f16_file)
            else:
                print(f"  ⚠ llama-quantize not found — keeping F16 GGUF")
                gguf_file = f16_file

    if os.path.isfile(gguf_file):
        size = os.path.getsize(gguf_file) / 1e9
        print(f"  ✓ GGUF exported: {gguf_file} ({size:.1f} GB)")
    else:
        print(f"  ✗ GGUF export failed")
    return gguf_file


def _export_gguf_via_library(merged_dir: str, gguf_file: str, quant_type: str):
    """Fallback GGUF export using the gguf Python library directly."""
    try:
        from gguf import GGUFWriter
        print("  gguf library available — but full conversion requires llama.cpp tools")
        print("  To install: pip install llama-cpp-python[server]")
        print("  Or build llama.cpp from source: https://github.com/ggerganov/llama.cpp")
        print(f"  Then run: convert-hf-to-gguf {merged_dir} --outfile {gguf_file} --outtype {quant_type.lower()}")
    except ImportError:
        print("  gguf library not installed. Install with: pip install gguf llama-cpp-python")
    raise RuntimeError(
        f"GGUF conversion requires llama.cpp tools which are not available. "
        f"Install llama-cpp-python or build llama.cpp from source."
    )


def push_to_hub(
    model_dir: str,
    repo_id: str,
    private: bool = True,
    gguf_file: str | None = None,
):
    """Push merged model (and optionally GGUF) to HuggingFace Hub."""
    from huggingface_hub import HfApi, upload_folder, upload_file

    api = HfApi()
    print(f"Pushing to HuggingFace Hub: {repo_id}")

    # Create repo if needed
    api.create_repo(repo_id, private=private, exist_ok=True)

    # Upload merged model
    print(f"  Uploading merged model from {model_dir} …")
    upload_folder(
        folder_path=model_dir,
        repo_id=repo_id,
        commit_message="Upload merged ApexIntel model",
    )
    print(f"  ✓ Model uploaded")

    # Upload GGUF if present
    if gguf_file and os.path.isfile(gguf_file):
        print(f"  Uploading GGUF: {gguf_file} …")
        upload_file(
            path_or_fileobj=gguf_file,
            path_in_repo=os.path.basename(gguf_file),
            repo_id=repo_id,
            commit_message=f"Upload GGUF: {os.path.basename(gguf_file)}",
        )
        print(f"  ✓ GGUF uploaded")

    print(f"  ✓ All pushed to https://huggingface.co/{repo_id}")


def main():
    parser = argparse.ArgumentParser(
        description="Export ApexIntel model — merge adapters, quantize, push"
    )
    parser.add_argument("--base-model", default=str(WORK / "models" / "base"),
                        help="Path to base model")
    parser.add_argument("--phase1-adapter",
                        default=str(WORK / "training" / "outputs" / "phase1_dapt" / "best_adapter"),
                        help="Path to Phase 1 LoRA adapter")
    parser.add_argument("--phase2-adapter",
                        default=str(WORK / "training" / "outputs" / "phase2_sft" / "best_adapter"),
                        help="Path to Phase 2 LoRA adapter")
    parser.add_argument("--merged-dir",
                        default=str(WORK / "training" / "outputs" / "merged"),
                        help="Output directory for merged model")
    parser.add_argument("--merge", action="store_true",
                        help="Merge adapters into base model")
    parser.add_argument("--gguf", type=str, default=None, choices=GGUF_QUANT_TYPES,
                        help="Export to GGUF with specified quantization")
    parser.add_argument("--gguf-dir",
                        default=str(WORK / "training" / "outputs" / "gguf"),
                        help="Output directory for GGUF files")
    parser.add_argument("--push", action="store_true",
                        help="Push to HuggingFace Hub")
    parser.add_argument("--repo", type=str, default=None,
                        help="HuggingFace repo ID (required with --push)")
    parser.add_argument("--private", action="store_true", default=True,
                        help="Make HF repo private")

    args = parser.parse_args()

    if not args.merge and not args.gguf and not args.push:
        parser.print_help()
        print("\nSpecify at least one action: --merge, --gguf, --push")
        sys.exit(1)

    gguf_file = None

    if args.merge:
        print("═" * 60)
        print("  Step 1: Merge LoRA Adapters")
        print("═" * 60)
        merge_adapters(
            base_model_path=args.base_model,
            phase1_adapter=args.phase1_adapter,
            phase2_adapter=args.phase2_adapter,
            output_dir=args.merged_dir,
        )

    if args.gguf:
        print("\n" + "═" * 60)
        print(f"  Step 2: Export GGUF ({args.gguf})")
        print("═" * 60)
        merged = args.merged_dir
        if not os.path.isdir(merged):
            print(f"ERROR: Merged model not found at {merged}")
            print("Run with --merge first, or specify --merged-dir")
            sys.exit(1)
        gguf_file = export_gguf(merged, args.gguf_dir, args.gguf)

    if args.push:
        print("\n" + "═" * 60)
        print("  Step 3: Push to HuggingFace Hub")
        print("═" * 60)
        if not args.repo:
            print("ERROR: --repo is required with --push")
            sys.exit(1)
        push_to_hub(args.merged_dir, args.repo, args.private, gguf_file)

    print("\n" + "═" * 60)
    print("  Export complete!")
    print("═" * 60)


if __name__ == "__main__":
    main()
