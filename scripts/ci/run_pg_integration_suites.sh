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
  --test warning_analysis_run_integration \
  --test notification_delivery_integration \
  --test event_outbox_integration \
  --test source_runtime_state_integration \
  --locked -- --ignored --test-threads=1

# ── Triage semantic dedup / warning ingress ──────────────────────────────────
run cargo test -p apex-triage --test triage_ingest_integration --locked -- --ignored --test-threads=1

# ── Notification delivery crash-window idempotency ───────────────────────────
# Receiver accepts -> settlement write lost -> lease expires -> row reclaimed:
# the redelivery must carry the same stable key (and the receiver dedupes it).
run cargo test -p apex-worker \
  --test notification_delivery_idempotency_integration \
  --locked -- --ignored --test-threads=1

# ── Insight evidence persistence (unit-test target) ──────────────────────────
run cargo test -p apex-insights --lib --locked -- --ignored --test-threads=1

# ── Canonical `app_users` login contract ─────────────────────────────────────
run cargo test -p apex-api --test app_users_login_integration --locked -- --ignored --test-threads=1

# ── Learning evaluation promotion gate ───────────────────────────────────────
# Frozen evaluation sets, versioned metric runs and the promotion/rejection
# rules (audit #38). DB-backed suites in this crate must be registered above;
# this command proves the evaluation gate itself runs in CI.
run cargo test -p apex-learning --features experimental --locked --no-fail-fast
