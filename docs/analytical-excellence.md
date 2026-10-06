# Analytical Excellence — the Foreign-Affairs-grade review pipeline

This document specifies the analytical-excellence layer: what it measures, how
it is enforced, how it improves over time, and the research it is built on.
All scores are computed deterministically from the text and evidence, without
asking an LLM to grade itself, and every verdict is persisted so quality is a
trend line rather than an opinion.

## 1. What "Foreign-Affairs-grade" means here

A product qualifies when all of the following hold at the publication
boundary (the editorial board, `crates/insights/src/analytical/editorial.rs`):

| Standard | Hard metric | Threshold |
|---|---|---|
| Argument integrity | zero hard violations (fabricated figures, unverified dates, out-of-range citations) | 0 |
| Factuality | weighted supported fraction of atomic claims | ≥ 0.75 |
| Depth | composite depth index over 10 measured dimensions | ≥ 0.50 |
| Reasoning warrant | argumentation warrant (weakest-link constrained) | ≥ 0.55 |
| Source independence | distinct-source-family score | ≥ 0.45 (primary registries exempt) |
| Uncertainty discipline | calibrated language, certainty penalized on thin evidence | enforced in the confidence gate |
| Red team | coordinated-placement detection over evidence | insight-level attack ≥ 0.6 blocks publish |
| Calibration | stated confidence mapped through the model's measured curve | applied when fitted |

Verdicts: `publish`, `revise` (publish with recalibrated confidence and the
`editorial:revise` tag), `reject` (never inserted; recorded in the ledger with
reasons).

## 2. The ten depth dimensions (measured, not vibes)

`crates/insights/src/analytical/depth.rs`, weights in parentheses:

1. causal depth (0.16) — causal-connector density with word-boundary counting;
2. counterargument depth (0.14) — alternative-explanation markers + attached
   hypotheses;
3. quantification (0.12) — numeric facts per 100 words (percentages, currency,
   counts; years excluded);
4. stakeholder coverage (0.10) — distinct actor classes from an 11-class
   taxonomy;
5. second-order effects (0.10) — consequence-chain markers;
6. uncertainty discipline (0.10) — calibrated-language presence minus
   certainty penalties scaled by corroboration;
7. source independence (0.10) — distinct source families × authority mix;
8. temporal depth (0.08) — historical baselines, lead/lag anchors;
9. actionability (0.05) — recommendations with actor + action + timing;
10. specificity (0.05) — proper-noun/entity density.

Filler/formulaic phrases are penalized. The tier ladder is
`shallow < desk < strategic < foreign_affairs`.

## 3. Atomic-claim verification (truth machinery)

`crates/insights/src/analytical/verification.rs` routes each claim by fact
type:

- **numbers** — percentages, currency (scaled), counts ≥ 10 are matched
  against evidence numbers within tolerance (10% relative / 0.5 absolute);
  unmatched figures in observed claims are *hard* violations; in inferences
  they are flagged but permitted (labelled projections);
- **dates** — past years asserted as observed must appear in the evidence;
  future years are projections;
- **citations** — ordinal must resolve within the supplied evidence;
- **causality** — observed causal claims need ≥ 2 evidence items or are
  downgraded to partially supported;
- **entities** — org-like proper nouns not present in the evidence are flagged
  (soft, heuristic);
- **recommendations** — exempt from support scoring but cannot smuggle
  fabricated figures (hard).

Weighted factuality: observed 1.0, inference 0.7 (cited) / partial (uncited,
explicitly hedged), unknown 0.3 with zero credit.

## 4. Argumentation warrants

`crates/insights/src/analytical/argumentation.rs` builds support/attack
arguments per claim: support from *independent* evidence families (duplicates
collapse — no corroboration illusion), attacks from violations, thin
causality, unhedged projections, unverified entities, and external red-team
findings. Warrants resolve saturatingly; the insight's overall warrant is
0.55·mean + 0.45·weakest-link, so one unsupported central claim cannot be
averaged away.

## 5. Calibration and fusion

`crates/insights/src/analytical/calibration.rs`:

- **noisy-OR** fusion for independent evidence chains (`combine_independent_evidence`);
- **isotonic (PAVA) recalibration** fitted from resolved predictions, with
  Brier, ECE, MCE and log loss as the hard error metrics;
- **competence gating** with Beta-shrunk domain weights, so small-sample
  domains cannot claim certainty.

The weekly self-improvement cycle resolves predictions against
**analyst-verified outcomes only** (reviewed warnings: `true_positive` /
`false_positive`), refits each model's curve, and stores it per model. The
released confidence is `calibrated × (0.80 + 0.20·discipline) × (1 − certainty
penalty)`, capped by evidence + warrant. Certainty injections strictly lower
released confidence — verified by fuzzing.

## 6. Measurable improvement (the trend contract)

Every product writes one row to `analytical_quality_scores` (migration 109)
with model/provenance stamps. Weekly:

- an aggregate snapshot (`analytical_quality_snapshots`) records mean/median
  depth, factuality, warrant, verdict mix and hard violations;
- **regression gate**: a > 0.05 drop in mean depth or mean factuality versus
  the previous snapshot emits `analytical_quality_regression` (audit event +
  warn) and degrades the weekly cycle;
- per-model summaries (`model_quality_summary`) and Brier comparisons make
  model swaps measurable.

Raw rows are retention-pruned (default 365 days,
`APEX_QUALITY_RETENTION_DAYS`, floor 30); snapshots are permanent, so years of
growth accumulate as a compact trend rather than unbounded raw detail.

## 7. Research basis (2025–2026)

- **ComInsight** (`arXiv:2610.03525`) — atomic insights composed into
  higher-order conclusions with provenance; drives the claims→warrant→review
  stack.
- **FActScore** (`arXiv:2305.14251`), **Chain-of-Verification**
  (`arXiv:2309.11495`), **FinGround** (`arXiv:2604.23588`) — atomic fact
  scoring and type-routed verification; drives §3.
- **Contestable multi-agent debate / QBAF** (`arXiv:2605.14495`) — support and
  attack arguments with provenance; drives §4.
- **Silent Dissent** (`arXiv:2610.02702`) — debate consensus can overstate
  agreement, so dissent/uncertainty is preserved rather than voted away.
- **CHAIN** (`arXiv:2609.36689`) — causal-chain fusion with noisy-OR
  aggregation; drives §5.
- **Corroboration Illusion** (`arXiv:2609.22246`) — corpus poisoning inflates
  apparent consensus; drives duplicate-collapsing and placement detection.
- **Competence-gated pooling** (`arXiv:2609.12101`) and **TTCL**
  (`arXiv:2609.02695`) — measured competence over verbal confidence, test-time
  calibration; drives competence weights and the weekly refit.
- **ICD 203/206 analytic standards** (ODNI) — sourced, uncertainty expressed,
  assumptions vs judgments distinguished, alternatives analysed, clear
  argumentation; encoded across §2–§4.

## 8. Adversarial verification of the pipeline itself

`crates/worker/examples/analytical_adversarial_dogfood.rs` (CI:
`scripts/ci/check_analytical_dogfood.sh`) runs 12 handcrafted attacks plus 400
mutation-fuzz iterations. Current status: **all caught, 0 escapes, 0
monotonicity violations** — including the live-model probe where Qwen3-8B
repeated a poisoned rumor and the board rejected it with a hard violation.

Regenerate evidence: `cargo run -p apex-worker --example
analytical_adversarial_dogfood`; live probe with `APEX_DOGFOOD_LIVE=1
LLM_BASE_URL=… LLM_MODEL=…`.
