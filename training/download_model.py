from huggingface_hub import snapshot_download
print("Downloading Qwen/Qwen3-30B-A3B ...")
snapshot_download(
    "Qwen/Qwen3-30B-A3B",
    local_dir="/workspace/models/base",
    ignore_patterns=["*.gguf", "*.ggml"],
)
print("DONE - Model downloaded to /workspace/models/base")
