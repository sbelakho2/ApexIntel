#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   scripts/ops_prod_insight_quality_audit.sh [host] [audit_kind] [window_days] [recent_hours] [ssh_identity_file]
# Example:
#   scripts/ops_prod_insight_quality_audit.sh hetzner-apexintel both 60 2 ~/.ssh/hetzner-db-mac

HOST="${1:-hetzner-apexintel}"
AUDIT_KIND="${2:-both}"
WINDOW_DAYS="${3:-60}"
RECENT_HOURS="${4:-2}"
SSH_IDENTITY_FILE="${5:-${SSH_IDENTITY_FILE:-}}"

if ! [[ "$WINDOW_DAYS" =~ ^[0-9]+$ ]]; then
  echo "window_days must be an integer, got: $WINDOW_DAYS" >&2
  exit 1
fi

if ! [[ "$RECENT_HOURS" =~ ^[0-9]+$ ]]; then
  echo "recent_hours must be an integer, got: $RECENT_HOURS" >&2
  exit 1
fi

case "$AUDIT_KIND" in
  dns-commercial|cert-commercial|both)
    ;;
  *)
    echo "audit_kind must be one of: dns-commercial, cert-commercial, both" >&2
    exit 1
    ;;
esac

echo "[ops] host=$HOST audit_kind=$AUDIT_KIND window_days=$WINDOW_DAYS recent_hours=$RECENT_HOURS"

run_psql() {
  local ssh_args=(-o ServerAliveInterval=20 -o ServerAliveCountMax=3)
  if [[ -n "$SSH_IDENTITY_FILE" ]]; then
    ssh_args+=(-i "$SSH_IDENTITY_FILE")
  fi

  ssh "${ssh_args[@]}" "$HOST" \
    'DB_URL=$(sudo grep "^DATABASE_URL=" /opt/apexintel/config/.env | cut -d= -f2-); psql "$DB_URL" -v ON_ERROR_STOP=1 -P pager=off -f -'
}

if [[ "$AUDIT_KIND" == "dns-commercial" || "$AUDIT_KIND" == "both" ]]; then
  run_psql <<SQL
\echo '=== dns_domain_exaggeration_summary ==='
WITH candidates AS (
  SELECT created_at, insight_type, title, summary
  FROM insights
  WHERE created_at > NOW() - INTERVAL '${WINDOW_DAYS} days'
    AND lower(coalesce(insight_type, '')) IN ('security_compliance', 'cybersecurity_threat', 'llm_security_compliance', 'llm_cybersecurity_threat')
    AND (
      lower(coalesce(summary, '')) LIKE '%dkim%'
      OR lower(coalesce(summary, '')) LIKE '%dns%'
      OR lower(coalesce(summary, '')) LIKE '%domain%'
      OR lower(coalesce(summary, '')) LIKE '%lookalike%'
      OR lower(coalesce(summary, '')) LIKE '%spoof%'
    )
    AND (
      lower(coalesce(summary, '')) LIKE '%as9100%'
      OR lower(coalesce(summary, '')) LIKE '%iso 13485%'
      OR lower(coalesce(summary, '')) LIKE '%iatf 16949%'
      OR lower(coalesce(summary, '')) LIKE '%customer%'
      OR lower(coalesce(summary, '')) LIKE '%supply chain%'
      OR lower(coalesce(summary, '')) LIKE '%program%'
      OR lower(coalesce(summary, '')) LIKE '%qualification%'
      OR lower(coalesce(summary, '')) LIKE '%production delay%'
      OR lower(coalesce(summary, '')) LIKE '%commercial%'
    )
)
SELECT COUNT(*) AS total_rows,
       COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '${RECENT_HOURS} hours') AS rows_last_recent_window
FROM candidates;

\echo '=== dns_domain_exaggeration_by_type ==='
WITH candidates AS (
  SELECT insight_type
  FROM insights
  WHERE created_at > NOW() - INTERVAL '${WINDOW_DAYS} days'
    AND lower(coalesce(insight_type, '')) IN ('security_compliance', 'cybersecurity_threat', 'llm_security_compliance', 'llm_cybersecurity_threat')
    AND (
      lower(coalesce(summary, '')) LIKE '%dkim%'
      OR lower(coalesce(summary, '')) LIKE '%dns%'
      OR lower(coalesce(summary, '')) LIKE '%domain%'
      OR lower(coalesce(summary, '')) LIKE '%lookalike%'
      OR lower(coalesce(summary, '')) LIKE '%spoof%'
    )
    AND (
      lower(coalesce(summary, '')) LIKE '%as9100%'
      OR lower(coalesce(summary, '')) LIKE '%iso 13485%'
      OR lower(coalesce(summary, '')) LIKE '%iatf 16949%'
      OR lower(coalesce(summary, '')) LIKE '%customer%'
      OR lower(coalesce(summary, '')) LIKE '%supply chain%'
      OR lower(coalesce(summary, '')) LIKE '%program%'
      OR lower(coalesce(summary, '')) LIKE '%qualification%'
      OR lower(coalesce(summary, '')) LIKE '%production delay%'
      OR lower(coalesce(summary, '')) LIKE '%commercial%'
    )
)
SELECT insight_type, COUNT(*) AS row_count
FROM candidates
GROUP BY 1
ORDER BY row_count DESC, insight_type;

\echo '=== dns_domain_exaggeration_latest ==='
SELECT created_at,
       insight_type,
       LEFT(title, 120) AS title,
       LEFT(regexp_replace(coalesce(summary, ''), E'[\n\r\t]+', ' ', 'g'), 260) AS summary_preview
FROM insights
WHERE created_at > NOW() - INTERVAL '${WINDOW_DAYS} days'
  AND lower(coalesce(insight_type, '')) IN ('security_compliance', 'cybersecurity_threat', 'llm_security_compliance', 'llm_cybersecurity_threat')
  AND (
    lower(coalesce(summary, '')) LIKE '%dkim%'
    OR lower(coalesce(summary, '')) LIKE '%dns%'
    OR lower(coalesce(summary, '')) LIKE '%domain%'
    OR lower(coalesce(summary, '')) LIKE '%lookalike%'
    OR lower(coalesce(summary, '')) LIKE '%spoof%'
  )
  AND (
    lower(coalesce(summary, '')) LIKE '%as9100%'
    OR lower(coalesce(summary, '')) LIKE '%iso 13485%'
    OR lower(coalesce(summary, '')) LIKE '%iatf 16949%'
    OR lower(coalesce(summary, '')) LIKE '%customer%'
    OR lower(coalesce(summary, '')) LIKE '%supply chain%'
    OR lower(coalesce(summary, '')) LIKE '%program%'
    OR lower(coalesce(summary, '')) LIKE '%qualification%'
    OR lower(coalesce(summary, '')) LIKE '%production delay%'
    OR lower(coalesce(summary, '')) LIKE '%commercial%'
  )
ORDER BY created_at DESC
LIMIT 40;
SQL
fi

if [[ "$AUDIT_KIND" == "cert-commercial" || "$AUDIT_KIND" == "both" ]]; then
  run_psql <<SQL
\echo '=== certification_warning_exaggeration_summary ==='
WITH candidates AS (
  SELECT created_at, insight_type, title, summary
  FROM insights
  WHERE created_at > NOW() - INTERVAL '${WINDOW_DAYS} days'
    AND lower(coalesce(summary, '')) LIKE '%certif%'
    AND (
      lower(coalesce(summary, '')) LIKE '%warning%'
      OR lower(coalesce(summary, '')) LIKE '%outdated%'
      OR lower(coalesce(summary, '')) LIKE '%flagged%'
      OR lower(coalesce(summary, '')) LIKE '%expired%'
    )
    AND (
      lower(coalesce(summary, '')) LIKE '%customer%'
      OR lower(coalesce(summary, '')) LIKE '%qualification%'
      OR lower(coalesce(summary, '')) LIKE '%program%'
      OR lower(coalesce(summary, '')) LIKE '%supply chain%'
      OR lower(coalesce(summary, '')) LIKE '%commercial%'
      OR lower(coalesce(summary, '')) LIKE '%b2b%'
      OR lower(coalesce(summary, '')) LIKE '%opportunit%'
    )
)
SELECT COUNT(*) AS total_rows,
       COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '${RECENT_HOURS} hours') AS rows_last_recent_window
FROM candidates;

\echo '=== certification_warning_exaggeration_by_type ==='
WITH candidates AS (
  SELECT insight_type
  FROM insights
  WHERE created_at > NOW() - INTERVAL '${WINDOW_DAYS} days'
    AND lower(coalesce(summary, '')) LIKE '%certif%'
    AND (
      lower(coalesce(summary, '')) LIKE '%warning%'
      OR lower(coalesce(summary, '')) LIKE '%outdated%'
      OR lower(coalesce(summary, '')) LIKE '%flagged%'
      OR lower(coalesce(summary, '')) LIKE '%expired%'
    )
    AND (
      lower(coalesce(summary, '')) LIKE '%customer%'
      OR lower(coalesce(summary, '')) LIKE '%qualification%'
      OR lower(coalesce(summary, '')) LIKE '%program%'
      OR lower(coalesce(summary, '')) LIKE '%supply chain%'
      OR lower(coalesce(summary, '')) LIKE '%commercial%'
      OR lower(coalesce(summary, '')) LIKE '%b2b%'
      OR lower(coalesce(summary, '')) LIKE '%opportunit%'
    )
)
SELECT insight_type, COUNT(*) AS row_count
FROM candidates
GROUP BY 1
ORDER BY row_count DESC, insight_type;

\echo '=== certification_warning_exaggeration_latest ==='
SELECT created_at,
       insight_type,
       LEFT(title, 120) AS title,
       LEFT(regexp_replace(coalesce(summary, ''), E'[\n\r\t]+', ' ', 'g'), 260) AS summary_preview
FROM insights
WHERE created_at > NOW() - INTERVAL '${WINDOW_DAYS} days'
  AND lower(coalesce(summary, '')) LIKE '%certif%'
  AND (
    lower(coalesce(summary, '')) LIKE '%warning%'
    OR lower(coalesce(summary, '')) LIKE '%outdated%'
    OR lower(coalesce(summary, '')) LIKE '%flagged%'
    OR lower(coalesce(summary, '')) LIKE '%expired%'
  )
  AND (
    lower(coalesce(summary, '')) LIKE '%customer%'
    OR lower(coalesce(summary, '')) LIKE '%qualification%'
    OR lower(coalesce(summary, '')) LIKE '%program%'
    OR lower(coalesce(summary, '')) LIKE '%supply chain%'
    OR lower(coalesce(summary, '')) LIKE '%commercial%'
    OR lower(coalesce(summary, '')) LIKE '%b2b%'
    OR lower(coalesce(summary, '')) LIKE '%opportunit%'
  )
ORDER BY created_at DESC
LIMIT 40;
SQL
fi