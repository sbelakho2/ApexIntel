#!/usr/bin/env bash
# Live source-health audit: runs the source-pipeline dogfood against real
# runtime state, generated warnings, and a paced endpoint sample.
#
# Usage (on a host with DATABASE_URL and crawl egress, e.g. the production
# server):
#   DATABASE_URL=... bash scripts/ops/source_health_audit.sh [--live] [--sample N]
#
# Evidence is appended to release-evidence/source-health/audit-<utc>.txt when
# that directory exists (it is deployed with the repo checkout on the server).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."
: "${DATABASE_URL:?DATABASE_URL must be set}"

LIVE_ARGS=()
if [[ "${1:-}" == "--live" ]]; then
  LIVE_ARGS=(--live --sample "${2:-40}")
fi

mkdir -p release-evidence/source-health
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
OUT="release-evidence/source-health/audit-${STAMP}.txt"

{
  echo "source health audit $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "database: ${DATABASE_URL%%@*}@<redacted>"
  cargo run --locked -q -p apex-worker --example source_pipeline_dogfood -- \
    --db "$DATABASE_URL" "${LIVE_ARGS[@]}"
} | tee "$OUT"

echo "evidence: $OUT"
