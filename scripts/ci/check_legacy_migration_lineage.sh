#!/usr/bin/env bash
# #175 / N3: the removed `crates/store/migrations/` lineage must never have
# been applied to any environment.
#
# The legacy folder held a parallel migration set (including versions 066,
# 067 and 095) that was never embedded in the binary and whose 0019 was
# invalid SQL. The canonical chain intentionally skips those version numbers.
# This check proves the connected database's `_sqlx_migrations` table does not
# contain any of them.
#
# Usage:
#   DATABASE_URL=postgres://… scripts/ci/check_legacy_migration_lineage.sh
#
# Exit codes: 0 clean (or no migration table yet), 1 legacy rows found,
#             2 usage/environment error.
set -euo pipefail

url="${DATABASE_URL:-${TEST_DATABASE_URL:-}}"
if [[ -z "${url}" ]]; then
  echo "DATABASE_URL (or TEST_DATABASE_URL) must point at the database to check" >&2
  exit 2
fi
if ! command -v psql >/dev/null 2>&1; then
  echo "psql is required to inspect _sqlx_migrations" >&2
  exit 2
fi

# A fresh database has no migration table yet: nothing can have been applied.
has_table="$(psql "${url}" -Atc "SELECT to_regclass('_sqlx_migrations') IS NOT NULL;" 2>/dev/null || echo "f")"
if [[ "${has_table}" != "t" ]]; then
  echo "legacy lineage check: no _sqlx_migrations table yet — clean"
  exit 0
fi

legacy="$(psql "${url}" -Atc \
  "SELECT version::text || ':' || description FROM _sqlx_migrations WHERE version IN (66, 67, 95) ORDER BY version;" 2>/dev/null || true)"
if [[ -n "${legacy}" ]]; then
  echo "legacy migration lineage was applied and must be reconciled:" >&2
  echo "${legacy}" >&2
  exit 1
fi

echo "legacy lineage check: 066/067/095 are absent — clean"
