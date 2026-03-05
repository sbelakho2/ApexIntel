#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   scripts/ops_worker_health_snapshot.sh [host] [hours] [insight_minutes]
# Example:
#   scripts/ops_worker_health_snapshot.sh hetzner-apexintel 6 90

HOST="${1:-hetzner-apexintel}"
HOURS="${2:-6}"
INSIGHT_MINUTES="${3:-90}"

if ! [[ "$HOURS" =~ ^[0-9]+$ ]]; then
  echo "hours must be an integer, got: $HOURS" >&2
  exit 1
fi

if ! [[ "$INSIGHT_MINUTES" =~ ^[0-9]+$ ]]; then
  echo "insight_minutes must be an integer, got: $INSIGHT_MINUTES" >&2
  exit 1
fi

echo "[ops] host=$HOST hours=$HOURS insight_minutes=$INSIGHT_MINUTES"

ssh -o ServerAliveInterval=20 -o ServerAliveCountMax=3 "$HOST" \
  'DB_URL=$(sudo grep "^DATABASE_URL=" /opt/apexintel/config/.env | cut -d= -f2-); psql "$DB_URL" -v ON_ERROR_STOP=1 -P pager=off -f -' <<SQL
\echo '=== snapshot_meta ==='
SELECT now() AS snapshot_utc,
       now() - INTERVAL '${HOURS} hours' AS window_start_utc,
       now() - INTERVAL '${INSIGHT_MINUTES} minutes' AS insight_window_start_utc;

\echo '=== trigger_queue_unclaimed ==='
SELECT job_kind, COUNT(*) AS unclaimed
FROM worker_trigger_queue
WHERE claimed_at IS NULL
GROUP BY 1
ORDER BY unclaimed DESC, job_kind;

\echo '=== trigger_queue_recent_activity ==='
SELECT
  COUNT(*) FILTER (WHERE requested_at > NOW() - INTERVAL '${HOURS} hours') AS queued_last_window,
  COUNT(*) FILTER (WHERE claimed_at > NOW() - INTERVAL '${HOURS} hours') AS claimed_last_window,
  COUNT(*) FILTER (WHERE completed_at > NOW() - INTERVAL '${HOURS} hours') AS completed_last_window,
  COUNT(*) FILTER (WHERE completed_at > NOW() - INTERVAL '${HOURS} hours' AND error IS NOT NULL) AS failed_last_window
FROM worker_trigger_queue;

\echo '=== crawl_health_warning_counts ==='
SELECT
  COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '${HOURS} hours') AS crawl_health_warnings_last_window,
  COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '24 hours') AS crawl_health_warnings_last_24h
FROM warnings
WHERE warning_type = 'crawl_health';

\echo '=== latest_crawl_health_warnings ==='
SELECT created_at, severity, title, LEFT(COALESCE(description, ''), 220) AS description_preview
FROM warnings
WHERE warning_type = 'crawl_health'
ORDER BY created_at DESC
LIMIT 8;

\echo '=== crawl_observation_source_coverage ==='
SELECT
  COALESCE(provenance->>'source', 'unknown') AS source,
  COUNT(*) AS obs_count,
  MAX(ts_utc) AS latest_obs_utc
FROM observations
WHERE observation_type = 'web_change'
  AND ts_utc > NOW() - INTERVAL '${HOURS} hours'
GROUP BY 1
ORDER BY obs_count DESC, source
LIMIT 40;

\echo '=== insight_reasoning_markers ==='
WITH recent AS (
  SELECT summary
  FROM insights
  WHERE created_at > NOW() - INTERVAL '${INSIGHT_MINUTES} minutes'
)
SELECT
  COUNT(*) AS insight_count,
  COUNT(*) FILTER (
    WHERE strpos(summary, '[1]') > 0
       OR strpos(summary, '[2]') > 0
       OR strpos(summary, '[3]') > 0
  ) AS with_bracket_refs,
  COUNT(*) FILTER (
    WHERE lower(summary) LIKE '% because %'
       OR lower(summary) LIKE '% therefore %'
       OR lower(summary) LIKE '% as a result %'
       OR lower(summary) LIKE '% leads to %'
  ) AS with_causal_language,
  COUNT(*) FILTER (
    WHERE lower(summary) LIKE '% if % would %'
       OR lower(summary) LIKE '% if % could %'
  ) AS with_counterfactual,
  COUNT(*) FILTER (
    WHERE lower(summary) LIKE '% by %'
       OR lower(summary) LIKE '% within %'
       OR lower(summary) LIKE '% before %'
  ) AS with_deadline_language
FROM recent;

  \echo '=== llm_quality_reports_recent ==='
  SELECT
    created_at,
    insight_type,
    ROUND(COALESCE(confidence, 0)::numeric, 3) AS confidence,
    LEFT(title, 80) AS title,
    LEFT(summary, 280) AS summary_preview
  FROM insights
  WHERE created_at > NOW() - INTERVAL '${HOURS} hours'
    AND insight_type IN ('llm_eval_report', 'llm_self_improvement', 'llm_training_examples')
  ORDER BY created_at DESC
  LIMIT 20;

  \echo '=== llm_eval_latest_metrics ==='
  WITH latest_eval AS (
    SELECT created_at, summary
    FROM insights
    WHERE insight_type = 'llm_eval_report'
    ORDER BY created_at DESC
    LIMIT 1
  )
  SELECT
    created_at,
    NULLIF((regexp_match(summary, 'pass_rate=([0-9.]+)%'))[1], '')::numeric AS pass_rate_pct,
    NULLIF((regexp_match(summary, 'avg_judge_score=([0-9.]+)'))[1], '')::numeric AS avg_judge_score,
    NULLIF((regexp_match(summary, 'hallucination_rate=([0-9.]+)%'))[1], '')::numeric AS hallucination_rate_pct,
    NULLIF((regexp_match(summary, 'total_cases=([0-9]+)'))[1], '')::int AS total_cases,
    NULLIF((regexp_match(summary, 'failed_cases=([0-9]+)'))[1], '')::int AS failed_cases
  FROM latest_eval;

  \echo '=== llm_self_improvement_latest_metrics ==='
  WITH latest_cycle AS (
    SELECT created_at, summary
    FROM insights
    WHERE insight_type = 'llm_self_improvement'
    ORDER BY created_at DESC
    LIMIT 1
  )
  SELECT
    created_at,
    NULLIF((regexp_match(summary, 'captures_seeded=([0-9]+)'))[1], '')::int AS captures_seeded,
    NULLIF((regexp_match(summary, 'analysed=([0-9]+)'))[1], '')::int AS captures_analysed,
    NULLIF((regexp_match(summary, 'qualifying_examples=([0-9]+)'))[1], '')::int AS qualifying_examples,
    NULLIF((regexp_match(summary, 'avg_critique=([0-9.]+)'))[1], '')::numeric AS avg_critique,
    NULLIF((regexp_match(summary, 'prompt_improvements=([0-9]+)'))[1], '')::int AS prompt_improvements,
    NULLIF((regexp_match(summary, 'failure_hypotheses=([0-9]+)'))[1], '')::int AS failure_hypotheses,
    NULLIF((regexp_match(summary, 'training_examples=([0-9]+)'))[1], '')::int AS training_examples
  FROM latest_cycle;

  \echo '=== llm_quality_regression_warnings ==='
  SELECT
    warning_type,
    COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '${HOURS} hours') AS warnings_last_window,
    COUNT(*) FILTER (WHERE created_at > NOW() - INTERVAL '24 hours') AS warnings_last_24h,
    MAX(created_at) AS latest_warning_utc
  FROM warnings
  WHERE warning_type IN ('llm_quality_regression', 'llm_self_improvement_degradation')
  GROUP BY 1
  ORDER BY warning_type;

  \echo '=== latest_llm_quality_warnings ==='
  SELECT
    created_at,
    warning_type,
    severity,
    LEFT(title, 100) AS title,
    LEFT(COALESCE(description, ''), 280) AS description_preview
  FROM warnings
  WHERE warning_type IN ('llm_quality_regression', 'llm_self_improvement_degradation')
  ORDER BY created_at DESC
  LIMIT 10;

\echo '=== poi_discovery_tracks_recent ==='
SELECT
  COALESCE(metadata->>'discovery_track', 'unknown') AS discovery_track,
  COUNT(*) AS persons_created,
  COUNT(*) FILTER (WHERE COALESCE((metadata->>'seed_is_competitor')::boolean, false)) AS competitor_seeded
FROM persons
WHERE created_at > NOW() - INTERVAL '${HOURS} hours'
GROUP BY 1
ORDER BY persons_created DESC, discovery_track;

\echo '=== poi_artifact_social_onion_quality ==='
SELECT
  COALESCE(provenance->>'source', 'unknown') AS source,
  COUNT(*) AS artifact_count,
  ROUND(AVG(COALESCE(sentiment_score, 0))::numeric, 3) AS avg_confidence,
  MAX(ts_utc) AS latest_ts_utc
FROM poi_artifacts
WHERE ts_utc > NOW() - INTERVAL '${HOURS} hours'
  AND (
    COALESCE(provenance->>'source', '') LIKE 'social_%'
    OR COALESCE(provenance->>'source', '') LIKE 'darkweb_%'
  )
GROUP BY 1
ORDER BY artifact_count DESC, source;

\echo '=== latest_insight_samples ==='
SELECT
  created_at,
  LEFT(title, 100) AS title,
  LEFT(summary, 260) AS summary_preview
FROM insights
ORDER BY created_at DESC
LIMIT 8;
SQL
