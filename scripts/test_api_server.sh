#!/bin/bash
set -euo pipefail

: "${APEX_API_KEY:?Set APEX_API_KEY before running scripts/test_api_server.sh}"

KEY="${APEX_API_KEY}"
BASE="${BASE:-http://localhost:8080}"
ok=0
fail=0

test_route() {
  local path="$1"
  code=$(curl -s -o /dev/null -w "%{http_code}" "${BASE}${path}" -H "Authorization: Bearer ${KEY}")
  echo "${code}  ${path}"
  if [ "$code" -lt 500 ]; then ok=$((ok+1)); else fail=$((fail+1)); fi
}

test_route /api/health
test_route "/api/warnings?limit=1"
test_route "/api/insights?limit=1"
test_route /api/insights/weekly-memo
test_route "/api/companies?limit=1"
test_route "/api/persons?limit=1"
test_route "/api/search?q=test"
test_route /api/graph
test_route "/api/recipes?limit=1"
test_route /api/security
test_route "/api/sites?limit=1"
test_route "/api/capabilities?limit=1"
test_route "/api/certifications?limit=1"
test_route "/api/observations?limit=1"
test_route "/api/product-families?limit=1"
test_route "/api/logistics-nodes?limit=1"
test_route "/api/regulations?limit=1"
test_route "/api/poi-artifacts?limit=1"
test_route /api/dashboard
test_route "/api/competitors?limit=1"
test_route "/api/recipes/staging?limit=1"
test_route /api/security/dns-posture
test_route /api/security/lookalike-domains
test_route /api/security/kev-relevance
test_route /api/admin/crawl-status
test_route /api/admin/recipe-performance
test_route /api/admin/poi-coverage
test_route /api/graph/neighborhood/00000000-0000-0000-0000-000000000001
test_route /api/graph/path/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002

echo ""
echo "${ok} OK, ${fail} FAILED out of $((ok+fail)) routes"
