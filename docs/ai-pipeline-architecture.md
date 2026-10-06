# ApexIntel AI Pipeline Architecture — Constant Learning, Constant Testing, Constant Improving

This document is the authoritative description of the AI/analysis pipeline. It
surveys the candidate paradigm space (deliberately **not** limited to RAG/CAG),
states what ApexIntel selected and why, maps every piece to code, and lists the
enforcement points that make the quality loop constant.

---

## 1. Paradigm survey (full space, with verdicts)

| Paradigm | What it is | ApexIntel verdict |
|---|---|---|
| **RAG** (retrieval-augmented generation) | Retrieve top-k context from a vector/knowledge store, inject into the prompt | **In use, narrow**: `crates/llm/src/rag.rs` for knowledge-base grounding; citations enforced (`citation_from_response`) |
| **CAG** (cache-augmented generation) | Preload a compact knowledge snapshot so generation needs no live retrieval | **Added (2026-10)**: `llm::cache` deterministic work cache (SHA-256 workflow/model/prompt/evidence key) is the CAG layer's persistence half; the weekly SelfImprovementCycle reuses cached contexts so repeated eval is not repeated inference |
| **KAG / GraphRAG** | Ground generation in a knowledge *graph* instead of flat passages | **In use**: `crates/graph` (entity/relationship graph, edge expiry, influence scoring) feeds dossiers; the triage/dedup layer uses embeddings + graph proximity |
| **CRAG / corrective RAG** | Score retrieved context, discard unreliable chunks, re-retrieve | **In use**: `quality_assurance.rs` credibility scoring gates context before generation; weak signals are fused with explicit posture ("evidence posture") rather than silently trusted |
| **Self-RAG** | Model emits reflection tokens, cites sources inline | **Partial**: evidence-chain requirements (`evidence_chain.rs`), citation grounding, and the devil's-advocate bias challenge (`bias_mitigation.rs`) enforce self-checked output |
| **HyDE / query expansion** | Hypothetical-document embeddings for retrieval | Not selected (retrieval targets are typed registry signals, not open corpora) |
| **Fusion + reranking** | Multi-strategy retrieval with score fusion | **In use**: `correlation.rs` (120+ signal taxonomy, Granger + mutual information) and `cross_entity_correlation.rs` fuse signal domains into ranked correlations |
| **Agentic retrieval (ReAct, tools)** | LLM plans and calls tools in a loop | **Bounded use**: manual `HypothesisGeneration` job runs a bounded agent loop with read-only store tools; validation/staging identical to batch mode so agentic cannot bypass gates |
| **Reflection / critique (Reflexion)** | Generate → critique → regenerate | **In use**: `llm::self_improvement.rs` critique stage; weekly SelfImprovementCycle; eval-gate decisions |
| **Self-consistency / majority vote** | Sample N answers, take consensus | Partial: consensus calls exist but are opt-in (`#96/#97`); determinism preferred for auditability |
| **GRPO/RLVR/DPO alignment loops** | Reward-driven post-training | **In use (offline)**: `training/` runs DAPT (QLoRA on 8×RTX 5090) + SFT (FSDP); reward signals come from analyst review labels exported as golden sets |
| **Continual fine-tuning (replay/EWC/LoRA)** | Keep fine-tuning without catastrophic forgetting | **In use**: `llm_training_datasets` (versioned, digest-signed) exports Alpaca-format examples only when measured score ≥ floor AND the output was used; training pipeline consumes them |
| **Active learning / uncertainty sampling** | Label the most informative examples | **In use**: triage queue is the sampling surface — lowest-confidence, highest-impact items surface first; analyst actions become training truth |
| **Evals-as-code (eval-driven development)** | Versioned eval sets, CI gates on pass rate | **In use**: `llm::evaluation.rs` suites + frozen-set machinery (`learning_eval_*`), golden-set regression, supplier-pricing dogfood eval |
| **Negative controls / red-teaming** | Shuffle-based null tests; adversarial probes | **In use + newly wired**: `learning::negative_control` now gates every mined candidate before staging (nightly PatternMining); `insights::adversarial.rs` red-teams analysis |
| **A/B and bandit rollout** | Shadow or split traffic between versions | Partial: calibration `PredictionBoard` compares measured telemetry before promotion; no traffic split (single-tenant product) |
| **Drift detection / monitoring** | Detect distribution shift, silent degradation | **In use**: `FeatureDriftCheck` job, silent-source outage detection, source health monitor, circuit breakers |
| **Memory layers (MemGPT-style)** | Persistent context windows across turns | **In use (scoped)**: `memo.rs`, dossier accumulation, `llm_cache` with expiry; unbounded memory deliberately rejected (hallucination risk) |
| **Test-time compute scaling** | Spend more inference at decision time | **Bounded use**: batch vs agentic mode (cost profile per job type); never unbounded |

The rejection of "just RAG" is principled: ApexIntel's domain is *typed signal
telemetry* (observation streams, registries, recipes), not open-text corpora.
The core loop is therefore **statistics-first** (`crates/learning`:
mine → hypothesize → backtest → negative controls → frozen-set promotion),
with LLM generation layered where narrative is needed — not the other way
around.

---

## 2. The three constant loops

### 2.1 Constant-learning (daily + weekly)

| Stage | Where | Schedule |
|---|---|---|
| Pattern mining (Fisher exact, FDR, stability) | `nightly.rs::run_pattern_mining` → `run_mining_pipeline` | Daily 02:00 |
| **Negative-control + walk-forward backtest gates** (new) | `nightly.rs::validate_mined_candidates` | Same job, before staging |
| Hypothesis generation + staging (review-only) | `agent_tools.rs::stage_hypothesis_results` | Same job |
| Cross-domain signal-combination mining | `intelligence.rs::run_cross_domain_mining` | Weekly |
| **Correlation persistence + deepening** (new) | `store::upsert_insight_correlations` + `correlation_recipe_definition` | Weekly, same job |
| Outcome tracking / source scoring | `intelligence.rs::run_outcome_tracking` | Weekly |
| Analyst feedback ingestion | API triage actions → `insight_feedback_events` (new wiring); warning reviews | Continuous |
| Self-improvement cycle (critique + dataset export) | `continuous_improvement.rs` | Weekly |

Learning is **never auto-firing**: staged recipes are a review/metadata store;
only lifecycle promotion (gated) makes them fire.

### 2.2 Constant-testing (every promotion is measured)

0. **Editorial board at publication** (new, 2026-10): every generated product
   passes `crates/insights/src/analytical` — 10-dimension depth scoring,
   type-routed atomic-claim verification (numbers/dates/citations/causality/
   entities, FActScore-style), argumentation warrants (weakest-link bound,
   duplicate sources collapsed), coordinated-placement red-teaming, per-model
   calibration, and a publish/revise/reject verdict. Scores land in
   `analytical_quality_scores` (migration 109) and the weekly review snapshots
   them, gates regressions (> 0.05) and refits each model's calibration curve
   from analyst-verified resolved predictions. See
   `docs/analytical-excellence.md` and `docs/model-portability.md`.


1. **Publication gate** (new): every LLM insight passes `quality_control::run_checks`
   (coherence ≥ 0.7, hallucination risk ≤ 0.3) plus a corroboration floor
   (≥ 2 distinct sources, except primary registries) before insert
   (`recipes.rs`).
2. **Promotion eval gate** (new): weekly PromotionBoard applies
   `evaluate_weekly_promotion` — this week's measured precision/FPR must be a
   statistically significant improvement over last week's on analyst-reviewed
   warnings, with zero critical regression, explicit training-truth opt-in, and
   a 10-review sample floor. Failures are audit events and fail closed.
3. **Frozen-set promotion** (`evaluate_promotion` / `evaluate_promotion_over_measured`):
   the full-rigor path for candidates with complete telemetry; requires every
   critical metric measured and a verifiable frozen set digest.
4. **Golden-set regression**: reviewed warnings are the golden set; ≥ 95%
   agreement required or a review warning emits.
5. **Negative controls**: shuffle-based permutation tests (time + entity) must
   pass for staging; walk-forward backtest (4 folds, expanding window) must
   pass too. Rejections are persisted with `passed_gates = false` — never
   silently dropped.
6. **Statistical gates** for recipes: 9 promotion gates (effect, significance,
   FDR, stability, entity coverage, negative control, FP budget, counterfactual).
7. **Eval suites + dogfood**: `standard_eval_suite`, weekly SelfImprovementCycle
   eval gate, supplier-pricing dogfood (63/63 band-correct on the last run).

### 2.3 Constant-improving (deepening)

1. **Correlation mining → recipes**: every weekly signal-combination discovery
   is persisted (`insight_correlations`, migration 108) and the top
   correlations are **staged as `corr_*` deep-insight recipes** for review
   (`intelligence.rs`).
2. **Curated deep-correlation seed recipes**: `CORR001–CORR012` in
   `config/recipes_seed.yaml` — 12 causal-chain recipes (conflict→energy cost,
   sanctions→FX, mineral restriction→cell cost, logistics→shortage,
   semiconductor allocation→harness RFQ, breach→quality escape, disaster→
   freight, patent cluster→capacity, hiring→expansion, tariff→nearshoring,
   allocation→pricing, tender→cell allocation).
3. **Threshold calibration**: `auto_calibrate_recipe_thresholds` (PromotionBoard
   only) rewrites activation thresholds from FP evidence.
4. **Adaptive firing**: `hydrate_feedback_tracker` feeds 90d feedback into
   per-recipe F1, fatigue suppression, and clamped adaptive thresholds.
5. **Recipe refinements**: `quality_assurance::generate_recipe_refinements`
   proposes threshold/narrative refinements from quality feedback.
6. **Dataset export loop**: approved `llm_training_datasets` → `training/`
   pipeline (DAPT/SFT) — the feedback loop closes into model weights.

---

## 3. Enforceability

- Every gate decision is persisted or audit-logged: mined-candidate rows carry
  `passed_gates`; promotion rejections write `recipe_promotion_gate_rejected`
  audit events; correlations carry `status` and `derived_recipe_codes`.
- Fail-closed semantics everywhere a gate cannot be evaluated (telemetry
  error ⇒ no promotion; unmeasured critical metric ⇒ rejection).
- CI guards (`.woodpecker.yml`) enforce the *code-level* invariants (fabricated
  metrics, false-success, migration lineage, gitleaks, cargo-deny, workspace
  tests, PG integration suites including the learning-eval gate).

## 4. Gaps deliberately closed by this change set

| Gap | Fix |
|---|---|
| Negative-control/backtest libraries had no caller | wired into nightly mining |
| `evaluate_promotion` had no job caller | weekly PromotionBoard eval gate |
| `triage::feedback_integration` was unwired | triage actions persist into `insight_feedback_events` |
| Correlations were compute-only (logged and dropped) | migration 108 + persistence + recipe deepening |
| Seed recipes had 16/389 CrossCorrelation recipes | +12 curated CORR recipes; `corr_*` auto-staging |
| CAG absent | deterministic work cache + context reuse documented as the CAG layer; RAG remains for knowledge grounding |
| No publication quality gate on the LLM path | coherence/hallucination/corroboration gate at insert |
