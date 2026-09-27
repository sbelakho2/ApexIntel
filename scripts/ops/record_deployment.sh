#!/usr/bin/env bash
# Record one production deployment's provenance.
#
# Every deploy must leave a durable record of: the git SHA, the CI pipeline that
# produced the artifact, the test result, the migration test result, the
# artifact digest, and the deploy time. The script prints the matching
# APEX_GIT_SHA / APEX_CI_PIPELINE_ID / APEX_ARTIFACT_DIGEST / APEX_DEPLOYED_AT
# exports; add them to the service environment and restart so `/api/version`
# and `/api/health/capabilities` report the exact running deployment.
#
# Usage:
#   scripts/ops/record_deployment.sh \
#     --git-sha "$(git rev-parse HEAD)" \
#     --pipeline-id "$CI_PIPELINE_NUMBER" \
#     --tests passed \
#     --migrations passed \
#     --artifact /opt/apexintel/bin/apex-api \
#     [--deployed-at 2026-09-26T21:00:00Z] \
#     [--ledger /opt/apexintel/deployments.jsonl]
#
# One JSON object per line is appended to the ledger (default:
# ./deployments.jsonl), which is the per-deployment audit trail.
set -euo pipefail

usage() {
  grep '^#' "$0" | sed 's/^# \{0,1\}//' | sed -n '2,26p'
  exit "${1:-0}"
}

ledger="${APEX_DEPLOYMENT_LEDGER:-deployments.jsonl}"
git_sha="${APEX_GIT_SHA:-${CI_COMMIT_SHA:-}}"
pipeline_id="${APEX_CI_PIPELINE_ID:-${CI_PIPELINE_NUMBER:-${CI_PIPELINE_ID:-}}}"
tests="${APEX_TEST_RESULT:-}"
migrations="${APEX_MIGRATION_TEST_RESULT:-}"
artifact_digest="${APEX_ARTIFACT_DIGEST:-}"
deployed_at="${APEX_DEPLOYED_AT:-}"
artifact_path=""
recorded_by="${APEX_DEPLOY_RECORDED_BY:-$(id -un 2>/dev/null || echo unknown)}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --ledger) ledger="$2"; shift 2 ;;
    --git-sha) git_sha="$2"; shift 2 ;;
    --pipeline-id) pipeline_id="$2"; shift 2 ;;
    --tests) tests="$2"; shift 2 ;;
    --migrations) migrations="$2"; shift 2 ;;
    --artifact) artifact_path="$2"; shift 2 ;;
    --artifact-digest) artifact_digest="$2"; shift 2 ;;
    --deployed-at) deployed_at="$2"; shift 2 ;;
    --recorded-by) recorded_by="$2"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) echo "unknown argument: $1" >&2; usage 2 ;;
  esac
done

if [[ -z "${git_sha}" ]]; then
  echo "--git-sha (or APEX_GIT_SHA/CI_COMMIT_SHA) is required" >&2
  exit 2
fi
if [[ -z "${pipeline_id}" ]]; then
  echo "--pipeline-id (or APEX_CI_PIPELINE_ID/CI_PIPELINE_NUMBER) is required" >&2
  exit 2
fi
if [[ -z "${tests}" ]]; then
  echo "--tests (passed|failed|skipped) is required" >&2
  exit 2
fi
if [[ -z "${migrations}" ]]; then
  echo "--migrations (passed|failed|skipped) is required" >&2
  exit 2
fi

if [[ -z "${artifact_digest}" ]]; then
  if [[ -z "${artifact_path}" ]]; then
    echo "provide --artifact <path> (digest computed) or --artifact-digest <digest>" >&2
    exit 2
  fi
  if [[ ! -f "${artifact_path}" ]]; then
    echo "artifact not found: ${artifact_path}" >&2
    exit 2
  fi
  if command -v sha256sum >/dev/null 2>&1; then
    artifact_digest="sha256:$(sha256sum "${artifact_path}" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    artifact_digest="sha256:$(shasum -a 256 "${artifact_path}" | awk '{print $1}')"
  else
    echo "neither sha256sum nor shasum is available to digest ${artifact_path}" >&2
    exit 2
  fi
fi

if [[ -z "${deployed_at}" ]]; then
  deployed_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
fi

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

mkdir -p "$(dirname "${ledger}")"
printf '{"git_sha":"%s","ci_pipeline_id":"%s","test_result":"%s","migration_test_result":"%s","artifact_digest":"%s","deployed_at":"%s","recorded_by":"%s"}\n' \
  "$(json_escape "${git_sha}")" \
  "$(json_escape "${pipeline_id}")" \
  "$(json_escape "${tests}")" \
  "$(json_escape "${migrations}")" \
  "$(json_escape "${artifact_digest}")" \
  "$(json_escape "${deployed_at}")" \
  "$(json_escape "${recorded_by}")" >> "${ledger}"

echo "Recorded deployment in ${ledger}:"
echo "  git_sha=${git_sha}"
echo "  ci_pipeline_id=${pipeline_id}"
echo "  test_result=${tests}"
echo "  migration_test_result=${migrations}"
echo "  artifact_digest=${artifact_digest}"
echo "  deployed_at=${deployed_at}"
echo
echo "Export these in the service environment so /api/version reports them:"
echo "  APEX_GIT_SHA=${git_sha}"
echo "  APEX_CI_PIPELINE_ID=${pipeline_id}"
echo "  APEX_ARTIFACT_DIGEST=${artifact_digest}"
echo "  APEX_DEPLOYED_AT=${deployed_at}"
