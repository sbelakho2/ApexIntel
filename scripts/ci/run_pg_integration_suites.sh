#!/usr/bin/env bash
# Canonical runner for every `#[ignore = "requires PostgreSQL"]` integration
# suite in the workspace.
#
# CI invokes this script from the `migrations` step (.woodpecker.yml) against a
# real PostgreSQL service; `npm run test:pg` invokes it locally. The script is
# the single source of truth for the PostgreSQL suite list, and
# `scripts/ci/check_pg_test_coverage.sh` fails the build whenever a new ignored
# PostgreSQL test target is not registered here.
#
# Order matters: migrations_integration bootstraps the schema first; every
# subsequent suite runs against the same migrated database, exactly as
# production starts against a migrated schema.
#
# Requirements: TEST_DATABASE_URL (or DATABASE_URL) must point at a database
# with the `vector`, `pg_trgm`, `btree_gist` and `uuid-ossp` extensions
# available (the CI service uses timescale/timescaledb:2.30.1-pg16).
#
# SAFETY: these suites are destructive (they delete from `event_outbox`,
# `app_users`, subscriptions and similar tables). The runner refuses to start
# unless the database name ends in `_ci` or `_test`, or the operator sets
# `APEX_TEST_PG_DESTRUCTIVE=1` to explicitly accept data loss.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

if [[ -z "${TEST_DATABASE_URL:-}" ]]; then
  if [[ -n "${DATABASE_URL:-}" ]]; then
    export TEST_DATABASE_URL="${DATABASE_URL}"
  else
    echo "TEST_DATABASE_URL (or DATABASE_URL) must be set to run the PostgreSQL integration suites" >&2
    exit 1
  fi
fi

# Never point the destructive suites at a database that is not obviously
# disposable. Parse the database name out of the connection URL.
db_name="${TEST_DATABASE_URL%%\?*}"
db_name="${db_name%%#*}"
db_name="${db_name##*/}"
case "${db_name}" in
  *_ci | *_test) ;;
  *)
    if [[ "${APEX_TEST_PG_DESTRUCTIVE:-0}" != "1" ]]; then
      echo "refusing to run destructive PostgreSQL suites against '${db_name}':" >&2
      echo "  use a database whose name ends in _ci or _test, or set" >&2
      echo "  APEX_TEST_PG_DESTRUCTIVE=1 if you really intend to destroy its data." >&2
      exit 1
    fi
    ;;
esac

run() {
  echo "+ $*"
  "$@"
}

# ── Schema bootstrap and store-level invariants ──────────────────────────────
# migrations_integration must run first: it proves the full migration chain
# applies to an empty database. The remaining store suites assert RLS scoping,
# alert subscriptions, the transactional outbox and source runtime state
# against that same schema.
run cargo test -p apex-store \
  --test migrations_integration \
  --test alert_subscriptions_integration \
  --test alert_subscription_crud_integration \
  --test rls_scoped_integration \
  --test auth_hardening_integration \
  --test insight_claim_evidence_integration \
  --test insight_analysis_run_integration \
  --test warning_analysis_run_integration \
  --test warning_evidence_integration \
  --test evidence_lineage_integration \
  --test notification_delivery_integration \
  --test event_outbox_integration \
  --test source_runtime_state_integration \
  --locked -- --ignored --test-threads=1

# ── Triage semantic dedup / warning ingress ──────────────────────────────────
run cargo test -p apex-triage --test triage_ingest_integration --locked -- --ignored --test-threads=1
run cargo test -p apex-triage --test semantic_dedup_pg_integration --locked -- --ignored --test-threads=1
run cargo test -p apex-triage --test router_integration_pg_integration --locked -- --ignored --test-threads=1

# ── Notification delivery crash-window idempotency ───────────────────────────
# Receiver accepts -> settlement write lost -> lease expires -> row reclaimed:
# the redelivery must carry the same stable key (and the receiver dedupes it).
run cargo test -p apex-worker \
  --test notification_delivery_idempotency_integration \
  --locked -- --ignored --test-threads=1

# ── Insight loaders: POI current_role (N6), newest-12 evidence (#99), ────────
# prompt-bound grounded LLM cache (#100) ─────────────────────────────────────
run cargo test -p apex-worker --test insight_poi_loading_integration --locked -- --ignored --test-threads=1

# ── POI person resolution: corroborated merges, warning/insight entity-array ─
# rewrites (#165), and same-source photo change detection (#164) ─────────────
run cargo test -p apex-worker --test poi_resolver_merge_integration --locked -- --ignored --test-threads=1

# ── Agentic hypothesis generation: real store tools + production staging ─────
# (W3) Valid final answers stage exactly one `staging` recipe; invalid answers
# stage none. Local runs use an isolated database (e.g. apexintel_ci_w3).
run cargo test -p apex-worker --test agentic_hypothesis_generation_integration --locked -- --ignored --test-threads=1

# ── Insight evidence persistence (unit-test target) ──────────────────────────
run cargo test -p apex-insights --lib --locked -- --ignored --test-threads=1

# ── Canonical `app_users` login contract ─────────────────────────────────────
run cargo test -p apex-api --test app_users_login_integration --locked -- --ignored --test-threads=1

# ── Browser session authority contract (audit P0-2) ──────────────────────────
# Disabled rows, stale session versions, role downgrade/promotion and legacy
# cookies, all resolved against the real `app_users` table.
run cargo test -p apex-api --test session_authority_integration --locked -- --ignored --test-threads=1

# ── Collaboration workspace authorization (audit #50/#51/#53/#54/#55/#58/#59/#60/#62) ──
# Workspace visibility/share/assignment matrix, list and activity-feed SQL
# filtering, owner-only annotation deletes, partial-PATCH column preservation,
# queue completion timestamps, assignment upsert and form validation.
run cargo test -p apex-api --test workspace_authorization_integration --locked -- --ignored --test-threads=1

# ── Seed bootstrap preserves authoritative recipe lifecycle ─────────────────
run cargo test -p apex-worker --test recipe_lifecycle_preservation_integration --locked -- --ignored --test-threads=1

# ── Recipe activation-threshold calibration tightens on high FPR ────────────
run cargo test -p apex-store --test recipe_calibration_integration --locked -- --ignored --test-threads=1

# ── Canonical POI priority: vector composite, filters, ordering ─────────────
run cargo test -p apex-store --test persons_priority_integration --locked -- --ignored --test-threads=1

# ── Alert routing policy: suppressed targets receive zero SSE events ────────
run cargo test -p apex-api --test alert_routing_policy_integration --locked -- --ignored --test-threads=1

# ── Login throttle: concurrent reserves serialize on the attempt key ────────
run cargo test -p apex-store --test login_throttle_pg_integration --locked -- --ignored --test-threads=1

# ── Battlecards: create outcomes, optimistic-concurrency edits, atomic regen ─
run cargo test -p apex-store --test battlecards_integration --locked -- --ignored --test-threads=1

# ── Whole-set readers cross the 500-row list clamp; page summaries are exact ─
run cargo test -p apex-store --test list_aggregates_integration --locked -- --ignored --test-threads=1

# ── Warning analysis consumes explicit warning_evidence links ────────────────
run cargo test -p apex-api --test warning_evidence_analysis_integration --features llm \
  --locked -- --ignored --test-threads=1

# ── Learning evaluation promotion gate ───────────────────────────────────────
# Frozen evaluation sets, versioned metric runs and the promotion/rejection
# rules (audit #38). DB-backed suites in this crate must be registered above;
# this command proves the evaluation gate itself runs in CI.
run cargo test -p apex-learning --features experimental --locked --no-fail-fast
