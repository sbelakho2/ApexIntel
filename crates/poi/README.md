# apex-poi

Person-of-Interest (POI) feature engineering, profile resolution, engagement plan generation, and profile updating.

## Responsibilities

- **Features** (`features.rs`): Compute behavioural feature vectors from artifacts and role history.
  - `compute_priority_vector(artifacts)` — derive a `PriorityVector` (cost/quality/speed/resilience/compliance/security) from artifact topics and types.
  - `compute_priority_vector_with_locale(artifacts, locale_keywords)` — locale-aware variant.
  - `infer_decision_style(pv)` — classify decision style from the dominant priority dimension.
  - `compute_influence_score(role_seniority, network_size, artifact_count)` — 0–100 influence score.
  - `role_seniority_score(title)` — map job title to 0.0–1.0 seniority weight.
  - `compute_pain_index(artifacts, now_utc)` — time-decayed pain index from recent artifacts.
  - `infer_change_appetite(role_history)` — classify propensity for change based on tenure patterns.
- **Model** (`model.rs`): `PriorityVector` scoring, `DecisionStyle`, `ChangeAppetite`, `InfluenceAssessment`.
- **Engagement** (`engagement.rs`): Generate engagement plans tailored to role, style, and pain level.
  - `generate_engagement_plan(profile, context)` — returns talking points, proof packs, and timing guidance.
- **Resolver** (`resolver.rs`): Deduplicate and merge person records from multiple sources.
- **Updater** (`updater.rs`): Apply incremental updates to `Person` records from new artifacts.

## Key design invariants

- All scores are clamped to `[0.0, 1.0]` or stated ranges; no silent saturation without log warning.
- `compute_pain_index` returns 0.0 for empty artifact lists — never panics.
- `role_seniority_score` returns 0.5 for unknown titles (conservative default).

## Engagement profile values

- `best_channel` is constrained to one of:
  - `direct_outreach`
  - `trade_show_referral`
  - `referral_trusted_partner`
  - `existing_relationship_only`
- `best_timing` values are intent labels (`immediately_pain_driven`, `budget_cycle_q4_q1`, `pre_audit_season`, `npi_phase_early`, `anytime_with_trigger`) used by downstream outreach orchestration.
- `recommended_proof_pack` is ordered by explicit POI proof preferences and may be empty when no preferences are present.
