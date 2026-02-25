# apex-recipes

Recipe lifecycle management — evaluation against signal gates, promotion, deprecation, and performance tracking.

## Responsibilities

- **Engine** (`engine.rs`): Core recipe evaluation against live observations.
  - `evaluate_recipe(recipe, observations, context)` — returns `RecipeEvalResult` with triggered/non-triggered evidence.
  - `score_evidence(slots)` — aggregate evidence confidence across evidence slots.
- **Gates** (`gates.rs`): Statistical quality gates that recipes must pass before promotion.
  - `RecipeGate` — evaluates a single gate: precision ≥ threshold, recall ≥ threshold, FPR ≤ threshold, min sample size.
  - `run_all_gates(recipe)` — returns pass/fail per gate with reason strings.
  - `passes_all_gates(recipe)` — boolean shortcut for promotion eligibility.
- **Lifecycle** (`lifecycle.rs`): State machine for recipe progression.
  - `stage_recipe(...)` — move a recipe from hypothesis to staged.
  - `promote_recipe(...)` — staged → production after passing all gates.
  - `deprecate_recipe(...)` — production → deprecated.
  - `archive_recipe(...)` — deprecated → archived.

## Recipe states

```
Hypothesis → Staged → Production → Deprecated → Archived
```

- Only `Staged` recipes with `precision`, `recall` measured over `min_sample_size` can be promoted.
- Deprecation triggers: FPR exceeded, prolonged inactivity, or precision trend declining.

## Key types

| Type | Description |
|------|-------------|
| `Recipe` | Canonical recipe definition (code, template, gates) |
| `RecipeEvalResult` | Evaluation outcome with evidence and score |
| `RecipeGate` | A single promotion quality gate |
| `RecipeStatus` | `Hypothesis / Staged / Production / Deprecated / Archived` |
