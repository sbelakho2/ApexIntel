#!/usr/bin/env bash
# Repo-wide dogfood: adversarial, assert-based checks for the pipelines whose
# silent failures have burned us before.
#
#   1. Analytical pipeline (depth, claim verification, editorial gates) —
#      deterministic attack suite + mutation fuzzing.
#   2. Source pipeline (registry, fetch policy, detection semantics, warning
#      hygiene) — registry invariants + policy assertions. The --db/--live
#      modes of the source harness run in ops (`scripts/ops/source_health_audit.sh`).
#
# Both are pure (no network, no database) and run in CI on every change.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

cargo run --locked -q -p apex-worker --example analytical_adversarial_dogfood
cargo run --locked -q -p apex-worker --example source_pipeline_dogfood
