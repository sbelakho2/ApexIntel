#!/usr/bin/env python3
"""Download the Qwen3-30B-A3B base model from HuggingFace Hub.
Supports retry logic, disk space checks, and authentication."""
import os
import sys
from pathlib import Path

try:
    from huggingface_hub import snapshot_download, login
except ImportError as e:
    print(f"ERROR: Required package not installed: {e}", flush=True)
    print("Install with: pip install huggingface_hub")
    sys.exit(1)

WORK = Path(__file__).resolve().parent.parent
MODEL_ID = os.environ.get("MODEL_ID", "Qwen/Qwen3-30B-A3B")
OUTPUT_DIR = os.environ.get("MODEL_DIR", str(WORK / "models" / "base"))
HF_TOKEN = os.environ.get("HF_TOKEN", "")

# Minimum required disk space in GB
MIN_DISK_GB = 150


def check_disk_space(path: str, min_gb: int) -> bool:
    """Check if there's enough free disk space at the given path."""
    try:
        import shutil
        usage = shutil.disk_usage(path)
        free_gb = usage.free / (1024 ** 3)
        if free_gb < min_gb:
            print(f"WARNING: Low disk space ({free_gb:.0f} GB free, need {min_gb} GB)", flush=True)
            return False
        print(f"  Disk space OK: {free_gb:.0f} GB free", flush=True)
        return True
    except Exception as e:
        print(f"  Could not check disk space: {e}", flush=True)
        return True  # Proceed anyway


def download_model() -> None:
    """Download the model with retry logic."""
    print(f"Downloading {MODEL_ID} ...", flush=True)
    print(f"  Output: {OUTPUT_DIR}", flush=True)

    # Authenticate if token provided
    if HF_TOKEN:
        try:
            login(token=HF_TOKEN, add_to_git_credential=False)
            print("  ✓ Authenticated with HuggingFace", flush=True)
        except Exception as e:
            print(f"  WARNING: HF login failed: {e}", flush=True)
            print("  Continuing without authentication (model may be gated)", flush=True)

    # Disk space check (parent directory of output)
    os.makedirs(OUTPUT_DIR, exist_ok=True)
    check_disk_space(str(Path(OUTPUT_DIR).parent), MIN_DISK_GB)

    # Download with retry
    max_retries = 3
    for attempt in range(1, max_retries + 1):
        try:
            snapshot_download(
                MODEL_ID,
                local_dir=OUTPUT_DIR,
                ignore_patterns=["*.gguf", "*.ggml", "*.ot", "*.msgpack"],
                resume_download=True,
                local_dir_use_symlinks=False,
            )
            print(f"DONE - Model downloaded to {OUTPUT_DIR}", flush=True)
            return
        except (OSError, IOError, ConnectionError) as e:
            print(f"ERROR (attempt {attempt}/{max_retries}): {e}", flush=True)
            if attempt < max_retries:
                import time
                wait = 2 ** attempt
                print(f"  Retrying in {wait}s ...", flush=True)
                time.sleep(wait)
            else:
                print(f"FATAL: Failed to download model after {max_retries} attempts", flush=True)
                sys.exit(1)
        except Exception as e:
            print(f"FATAL: Unexpected error during download: {e}", flush=True)
            sys.exit(1)


if __name__ == "__main__":
    download_model()
