# ═══════════════════════════════════════════════════════════════════
# ApexIntel Training Pipeline — Makefile
# ═══════════════════════════════════════════════════════════════════
#
# Quick reference:
#   make validate       — Pre-flight checks (no GPU needed)
#   make data           — Prepare training + eval data
#   make train          — Full training (Phase 1 + Phase 2)
#   make eval           — Run evaluation harness
#   make eval-dry       — Validate eval data (no model)
#   make eval-smoke     — Quick eval with 5 examples per file
#   make export         — Merge adapters → safetensors
#   make export-gguf    — Merge + quantize to GGUF Q4_K_M
#   make all            — Full pipeline end-to-end
#   make clean          — Remove generated training data
#
# Override workspace: make all WORK_DIR=/your/path
# ═══════════════════════════════════════════════════════════════════

SHELL      := /bin/bash
WORK_DIR   ?= $(shell pwd)
PYTHON     ?= python3
ACCELERATE ?= accelerate

# Convenience paths
TRAIN_DIR   := $(WORK_DIR)/training
OUTPUT_DIR  := $(TRAIN_DIR)/outputs
DATA_DIR    := $(TRAIN_DIR)/data
EVAL_DIR    := $(WORK_DIR)/training_data/evaluation
CONFIG_DIR  := $(TRAIN_DIR)/configs

# ── Phony targets ──────────────────────────────────────────────────────────
.PHONY: all validate data eval-data train train-dapt train-sft eval eval-dry eval-smoke export export-gguf clean help

# ── Default target ─────────────────────────────────────────────────────────
all: validate data train eval export
	@echo "═══════════════════════════════════ Pipeline complete"

# ── Help ───────────────────────────────────────────────────────────────────
help:
	@echo "ApexIntel Training Pipeline"
	@echo ""
	@echo "  make validate       Pre-flight checks"
	@echo "  make data           Prepare training + eval data"
	@echo "  make train          Phase 1 DAPT + Phase 2 SFT"
	@echo "  make train-dapt     Phase 1 DAPT only"
	@echo "  make train-sft      Phase 2 SFT only"
	@echo "  make eval           Full evaluation (requires model)"
	@echo "  make eval-dry       Validate eval data (no model)"
	@echo "  make eval-smoke     Quick eval (5 examples per file)"
	@echo "  make export         Merge adapters to safetensors"
	@echo "  make export-gguf    Merge + GGUF Q4_K_M"
	@echo "  make all            Full end-to-end pipeline"
	@echo "  make clean          Remove generated data"

# ── Pre-flight validation ──────────────────────────────────────────────────
validate:
	@echo "═══ Pre-flight validation ═══"
	$(PYTHON) $(TRAIN_DIR)/validate_pipeline.py --work-dir $(WORK_DIR)

# ── Data preparation ──────────────────────────────────────────────────────
data: training-data eval-data

training-data:
	@echo "═══ Preparing training data ═══"
	$(PYTHON) $(TRAIN_DIR)/prepare_data.py --work-dir $(WORK_DIR)

eval-data:
	@echo "═══ Generating eval scenarios (654+) ═══"
	$(PYTHON) $(EVAL_DIR)/generate_eval_data.py \
		--recipe-count 200 --poi-count 120 --memo-count 60 --entity-count 250

# ── Training ──────────────────────────────────────────────────────────────
train: train-dapt train-sft

train-dapt:
	@echo "═══ Phase 1: DAPT ═══"
	$(ACCELERATE) launch \
		--config_file $(CONFIG_DIR)/accelerate_8gpu.yaml \
		$(TRAIN_DIR)/train_phase1_dapt.py

train-sft:
	@echo "═══ Phase 2: SFT ═══"
	$(ACCELERATE) launch \
		--config_file $(CONFIG_DIR)/accelerate_8gpu.yaml \
		$(TRAIN_DIR)/train_phase2_sft.py

# ── Evaluation ────────────────────────────────────────────────────────────
eval:
	@echo "═══ Evaluation (full) ═══"
	$(PYTHON) $(TRAIN_DIR)/eval_harness.py \
		--out-report $(OUTPUT_DIR)/eval_report.json

eval-dry:
	@echo "═══ Evaluation dry run ═══"
	$(PYTHON) $(TRAIN_DIR)/eval_harness.py --dry-run \
		--out-report $(OUTPUT_DIR)/eval_dryrun.json

eval-smoke:
	@echo "═══ Evaluation smoke test (5 per file) ═══"
	$(PYTHON) $(TRAIN_DIR)/eval_harness.py \
		--max-examples 5 \
		--out-report $(OUTPUT_DIR)/eval_smoke.json

# ── Export ────────────────────────────────────────────────────────────────
export:
	@echo "═══ Export: merge adapters ═══"
	$(PYTHON) $(TRAIN_DIR)/export_model.py --merge

export-gguf:
	@echo "═══ Export: merge + GGUF Q4_K_M ═══"
	$(PYTHON) $(TRAIN_DIR)/export_model.py --merge --gguf Q4_K_M

export-push:
	@test -n "$(HF_REPO)" || (echo "ERROR: set HF_REPO=org/model-name" && exit 1)
	@echo "═══ Export: merge + push to $(HF_REPO) ═══"
	$(PYTHON) $(TRAIN_DIR)/export_model.py --merge --push --repo $(HF_REPO)

# ── Clean ─────────────────────────────────────────────────────────────────
clean:
	@echo "Cleaning generated training data …"
	rm -rf $(DATA_DIR)
	rm -rf $(OUTPUT_DIR)/eval_report.json $(OUTPUT_DIR)/eval_dryrun.json $(OUTPUT_DIR)/eval_smoke.json
	@echo "Done. (To also remove model outputs: rm -rf $(OUTPUT_DIR))"
