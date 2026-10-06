# ApexIntel Analysis Quality Standards

Measurable, enforceable thresholds for every stage of insight production.
Each standard names its enforcement point in code so an auditor can verify it
mechanically. "Unverifiable" is always treated as failure — never as pass.

## 1. Publication standards (per insight)

| # | Standard | Threshold | Enforcement |
|---|---|---|---|
| P1 | Coherence | ≥ 0.7 | `quality_control::run_checks` at insert (`recipes.rs` publication gate) |
| P2 | Hallucination risk | ≤ 0.3 | same gate |
| P3 | Corroboration | ≥ 2 distinct sources; exception: primary registries (sanctions/government/legal) | same gate (`distinct_source_count`) |
| P4 | Derived-only intelligence | never published | engine rejects `prior_warning_only` candidates |
| P5 | Evidence provenance | every claim cites evidence; citation deleted ⇒ claim downgraded | schema policy guard on `insight_claims` |
| P6 | Semantic dedup | no near-duplicate insights for an entity | `is_semantic_duplicate_recent_insight` + triage semantic dedup |
| P7 | Template leakage | no un-substituted `{{...}}` | `InsightCandidate::has_template_leakage` |
| P8 | Grounding citations | RAG responses must cite sources | `rag.rs::citation_from_response` |
| P9 | Bias challenge | devil's-advocate adjustment applied before confidence is stored | `apply_bias_mitigation` |

## 2. Recipe lifecycle standards

| # | Standard | Threshold | Enforcement |
|---|---|---|---|
| R1 | Promotion requires measured improvement | significant (z ≥ 1.96) improvement in precision/FPR over prior week | `evaluate_weekly_promotion` (weekly PromotionBoard) |
| R2 | Critical metrics never regress | FPR/grounding regression tolerance = 0.0 | same gate |
| R3 | Sample floor | ≥ 10 reviewed warnings (weekly gate), ≥ 30 (frozen-set gate) | promotion gates |
| R4 | Training truth | only explicit analyst confirmation may be training truth | `MetricObservation.is_truth_evidence` + DB constraint |
| R5 | Candidate staging | must pass negative controls + walk-forward backtest | `validate_mined_candidates` (nightly mining) |
| R6 | Statistical gates | effect > 1.5×, p < 0.01, FDR q < 0.05, stability ≥ 3/4 splits, entity coverage ≥ 5, FP budget ≤ 0.02, counterfactual ≥ 0.1 | `recipes::gates` |
| R7 | Deprecation | precision < 0.5 over 3 declining weeks or 8 inactive weeks | weekly deprecation policy |
| R8 | Calibration separation | thresholds only rewritten by PromotionBoard, never by monitoring | `calibration_eligible` |

## 3. Learning standards

| # | Standard | Threshold | Enforcement |
|---|---|---|---|
| L1 | Mining significance | Fisher p < 0.01, OR ≥ 1.5, stability ≥ 0.6, ≥ 5 entities | `miner::MinerConfig` |
| L2 | Multiple-comparison control | Benjamini-Hochberg FDR | `apply_fdr_correction` |
| L3 | Null-signal calibration | permutation p < 0.05 on time + entity shuffles | `negative_control` (now wired) |
| L4 | Temporal honesty | walk-forward backtest (expanding window) must pass | `backtest` (now wired) |
| L5 | Frozen-set integrity | runs only count on verifiable frozen sets with matching digests | `evaluate_promotion` |
| L6 | Rejection audit | rejected candidates persisted with `passed_gates = false` and reason | mining persistence |

## 4. Source standards

| # | Standard | Threshold | Enforcement |
|---|---|---|---|
| S1 | Zero policy exclusions | every known source registered and `enabled = true` | `scripts/ops/audit_source_exclusions.py` (exit 0) |
| S2 | Working-source proof | every source's reachability recorded with evidence | `scripts/verify_sources.py` → `release-evidence/source-verification/` |
| S3 | Validation evidence | a source is `operational` only after fetch + parser contract check | `source_is_validated` |
| S4 | Self-healing circuits | failures back off (30m→8h ladder), success resets, probe reopens | `record_source_attempt_failure` / breaker probe |
| S5 | Dark-web transport safety | only `.onion` hosts may traverse Tor; clearnet never downgraded to Tor and vice versa | `tor_client::fetch_onion_text` host guard |
| S6 | Outage honesty | "silent source" requires a recent successful fetch AND a fresh feed (`last_item_at` within 3d) while ingestion stopped; quiet feeds warn nothing; failing fetches report the real error in one consolidated warning (stable title) | `list_ingestion_stalled_sources` + `list_failing_sources` (migration 110) |
| S7 | Fetch compatibility | HTTP sources fetch with a browser User-Agent by default (CDNs mass-403 custom bot UAs even for public feeds); robots.txt and per-domain pacing still enforced; `APEX_CRAWL_USE_BOT_UA=1` opts out | `fetch_source` UA policy |
| S8 | Ingestion accounting | a source success means fetch + parser contract; observation inserts report actual new rows (`ON CONFLICT DO NOTHING` accounted separately); cycle logs `new/duplicates` counts | `insert_observation -> bool`, crawl summary |

## 5. Analytical standards (editorial board, migration 109)

| # | Standard | Threshold | Enforcement |
|---|---|---|---|
| A1 | Argument integrity | 0 hard violations (fabricated figures, unverified dates, out-of-range citations) | `editorial::review` — Reject |
| A2 | Factuality | weighted atomic-claim support ≥ 0.75 | same gate |
| A3 | Depth index | ≥ 0.50 across 10 measured dimensions (causal, counterargument, quantification, stakeholders, second-order, uncertainty, independence, temporal, actionability, specificity) | same gate; components persisted per product |
| A4 | Reasoning warrant | ≥ 0.55, weakest-link constrained | argumentation graph |
| A5 | Source independence | ≥ 0.45 (primary registries exempt) | duplicate-collapsing families |
| A6 | Calibration | isotonic curve fitted per model at ≥ 20 resolved predictions; Brier/ECE stored | weekly refit |
| A7 | Uncertainty discipline | certainty markers strictly lower released confidence | confidence gate + fuzz invariant |
| A8 | Red team | insight-level coordinated-placement attack ≥ 0.6 blocks publish | placement detection wired into review |
| A9 | Quality trend | > 0.05 drop in mean depth/factuality vs previous weekly snapshot ⇒ regression warning + degraded cycle | weekly review |
| A10 | Rejection audit | every reject persisted with verdict + reasons | quality ledger |
| A11 | Adversarial self-verification | analytical: 12 attack cases + 400 fuzz mutations, 0 escapes; source pipeline: registry floors, 0 excluded, UA policy, onion rules (CI) + DB/warning cross-checks and live endpoint audit (ops) | `scripts/ci/check_dogfood.sh`, `scripts/ops/source_health_audit.sh` |
| A12 | Model onboarding | candidate model: reachable, 0 hard violations, factuality ≥ 0.75 on its own output | `model_onboarding` example + registry gate |

## 6. Enforcement philosophy

1. **Fail closed**: missing telemetry, unmeasurable criticals, and store errors
   withhold promotion/publication, with audit events explaining why.
2. **Evidence first**: every verdict in this document is produced by a named
   function; nothing passes on silence.
3. **Review before fire**: staging is review-only; firing is lifecycle-driven.
4. **Reproducibility**: decisions are persisted (`insight_correlations`,
   `learning_eval_*`, `recipe_weekly_metrics`, audit log) so any promotion,
   rejection, or publication can be re-derived from data.
