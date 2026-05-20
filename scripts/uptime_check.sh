#!/usr/bin/env bash

set -euo pipefail

HEALTHCHECK_URL="${HEALTHCHECK_URL:-http://127.0.0.1:8080/api/health/deep}"
PAGE_WEBHOOK_URL="${PAGE_WEBHOOK_URL:-}"

payload=$(curl -fsS --max-time 15 "${HEALTHCHECK_URL}") || {
  if [[ -n "${PAGE_WEBHOOK_URL}" ]]; then
    curl -fsS -X POST -H 'Content-Type: application/json' \
      -d "{\"text\":\"ApexIntel uptime check failed for ${HEALTHCHECK_URL}\"}" \
      "${PAGE_WEBHOOK_URL}" >/dev/null || true
  fi
  echo "health check failed for ${HEALTHCHECK_URL}" >&2
  exit 1
}

echo "${payload}" | jq -e '.status' >/dev/null 2>&1 || echo "${payload}" | grep -qi '"status"[[:space:]]*:' || {
  if [[ -n "${PAGE_WEBHOOK_URL}" ]]; then
    curl -fsS -X POST -H 'Content-Type: application/json' \
      -d "{\"text\":\"ApexIntel health payload missing status: ${HEALTHCHECK_URL}\"}" \
      "${PAGE_WEBHOOK_URL}" >/dev/null || true
  fi
  echo "health payload missing status" >&2
  exit 1
}

echo "uptime check ok"