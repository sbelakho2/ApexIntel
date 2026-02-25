# apex-worker

Nightly and weekly pipeline orchestration — pure scheduling, stage processing, and report generation with no I/O.

## Responsibilities

- **Nightly pipeline** (`nightly.rs`): Crawl → Pattern Mining → POI Refresh → Feature Drift Check.
  - `CrawlStageResult`, `MiningStageResult`, `PoiRefreshStageResult`, `DriftCheckStageResult` — typed inputs.
  - `process_*_stage()` — pure stage processors producing `StageOutcome`.
  - `run_nightly_pipeline()` — full orchestration; emits structured tracing events.
  - `should_proceed()` — guards stage execution based on predecessor outcome.
  - `pipeline_health()` — 0.0–1.0 health score (skips count 0.5).
  - `NightlyReport` — aggregated report with `overall_success`, item/error totals, summary string.
- **Weekly pipeline** (`weekly.rs`): Promotion Board → Recipe Deprecation → Strategy Memo.
  - `evaluate_promotion()`, `run_promotion_board()` — promote staged recipes by precision/recall gates.
  - `evaluate_deprecation()`, `run_deprecation_check()` — deprecate recipes by FPR, inactivity, precision decline.
  - `build_memo_structure()` — construct the weekly intelligence brief.
  - `WeeklyReport` — aggregated weekly outcome.
- **Scheduler** (`scheduler.rs`): `Scheduler`, `JobDef`, `JobRun`, `JobStatus`, `Schedule`.
  - Pure-function scheduler (`is_job_due`) with jitter for thundering-herd prevention.
  - Custom job validation (`validate_custom_job`) with injection-safe allowlist.
- **Main** (`main.rs`): async entrypoint wiring the scheduler to real pipeline execution.

## Design principles

- All pipeline logic is `async`-free and I/O-free — inject pre-computed stage result structs.
- Structured `tracing::info!` events at every stage boundary for dashboards and alerting.
- `NightlyStage::all()` and `WeeklyStage::all()` enumerate stages in execution order; tests use golden JSON guards to detect accidental renames.

## Test coverage

```bash
cargo test -p apex-worker --lib
# ~199 tests: unit, golden JSON, E2E pipeline integration
```
