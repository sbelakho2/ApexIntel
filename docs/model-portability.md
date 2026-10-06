# Model Portability — swapping LLMs without losing the plot

The pipeline is built so a better model can be adopted in a day, with every
change measured before and after, and a one-command rollback. Nothing about
analytical quality is stored *inside* a model: depth scoring, claim
verification, argumentation and the editorial verdict are deterministic code,
so they evaluate any model's output identically.

## 1. What is model-specific (and how it is handled)

| Artifact | Model-specific? | Handling |
|---|---|---|
| Prompts | Partly | Prompts carry versioned contracts; output is validated structurally (claims, citations, JSON) rather than trusting any model's style |
| Confidence calibration | **Yes** | Each model gets its own isotonic curve in `llm_model_registry.calibration`, fitted weekly from *that model's* resolved predictions. A new model starts uncapped-raw (evidence-capped) until its curve is fitted (≥ 20 resolved predictions) |
| Quality ledger | Stamped | `analytical_quality_scores.model_id` lets any model be compared on identical standards |
| Routing | Env/config | `TieredModels` maps workflows (fast extraction → narrative → deep reasoning → final synthesis) to model ids; per-workflow overrides via env |
| Training data | Portable | `llm_training_datasets` are model-agnostic Alpaca JSONL |

## 2. Onboarding a candidate model

```bash
CANDIDATE_LLM_BASE_URL=http://host:8081 CANDIDATE_LLM_MODEL=NewModel.gguf \
  cargo run -q -p apex-worker --example model_onboarding
```

The onboarding gate scores: reachability, citation compliance, mean factuality
of the model's own output (verified by the same claim verifier), hard
violations, and latency. A model with fabricated content or factuality < 0.75
fails onboarding.

Then run the full adversarial dogfood live against it:

```bash
APEX_DOGFOOD_LIVE=1 LLM_BASE_URL=http://host:8081 LLM_MODEL=NewModel.gguf \
  cargo run -q -p apex-worker --example analytical_adversarial_dogfood
```

## 3. Registry and activation

Register the candidate with its eval block (the onboarding output prints the
SQL), then activate:

- `PgStore::upsert_llm_model_registry(id, provider, display_name, context_window, "candidate", eval_scores, calibration=None, notes)`
- `PgStore::activate_llm_model(id)` — atomically deprecates the previous
  active model (exactly one `active` row).
- Point routing at the new id (`LLM_*` env / `TieredModels` config), restart
  the worker.

## 4. Calibration transfer and catch-up

There is **no blind inheritance**: the new model's released confidence runs
raw-but-evidence-capped until its own curve fits. Catch-up speed:

1. predictions already being generated will resolve within their horizons
   (days–weeks) and the weekly review fits the curve at ≥ 20 resolved pairs;
2. until then, the evidence cap
   (`0.25 + 0.30·independence + 0.20·factuality + 0.25·warrant`) keeps
   confidence honest;
3. the weekly regression gate (`analytical_quality_regression`) compares the
   new model's mean depth/factuality against the previous snapshot — a
   regression degrades the cycle and is visible in audit.

## 5. Comparing and rolling back

- `PgStore::model_quality_summary(model_id)` — mean/median depth, factuality,
  warrant, verdict mix per model.
- `PgStore::model_brier_summary()` — Brier by model over resolved
  predictions.
- Rollback: `activate_llm_model(previous_id)` and repoint routing. Because
  calibration is per model, rollback instantly restores the previous model's
  measured confidence behavior.

## 6. Storage at scale

- Raw quality rows: retention-pruned (default 365 days, floor 30) via
  `prune_analytical_quality_scores`; weekly snapshots are permanent.
- Predictions: indexed for the resolution queue (partial index on
  `resolve_by WHERE unresolved`); expired predictions are excluded from
  calibration.
- Ledger growth is bounded by retention × daily insight volume; the
  permanent trend lives in one snapshot row per week.
