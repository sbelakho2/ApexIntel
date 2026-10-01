#!/usr/bin/env bash
# scripts/restore.sh — Restore ApexIntel from backup
# Usage: ./scripts/restore.sh /path/to/backup/directory
#
# The destructive steps below run under `set -e`: a failed terminate/drop/
# create/restore aborts the script with a non-zero exit instead of falling
# through to an unconditional "Restore complete."

set -euo pipefail

BACKUP_DIR="${1:?Usage: restore.sh /path/to/backup/dir}"

: "${DATABASE_URL:?Set DATABASE_URL before running scripts/restore.sh}"

if [[ ! -f "${BACKUP_DIR}/apexintel.pgdump" ]]; then
    echo "ERROR: ${BACKUP_DIR}/apexintel.pgdump not found"
    exit 1
fi

echo "[$(date)] Restoring ApexIntel from ${BACKUP_DIR}"

DB_URL="${DATABASE_URL}"

echo "[$(date)] WARNING: This will drop and recreate the apexintel database."
read -p "Continue? [y/N] " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
fi

# Derive the database name and a maintenance URL: DROP/CREATE DATABASE cannot
# run through a connection to the database being replaced.
DB_URL_NO_QUERY="${DB_URL%%\?*}"
DB_QUERY=""
if [[ "${DB_URL}" == *\?* ]]; then
    DB_QUERY="?${DB_URL#*\?}"
fi
DB_NAME="${DB_URL_NO_QUERY##*/}"
if [[ -z "${DB_NAME}" || "${DB_NAME}" == "${DB_URL_NO_QUERY}" ]]; then
    echo "ERROR: cannot derive a database name from DATABASE_URL"
    exit 1
fi
MAINT_URL="${DB_URL_NO_QUERY%/*}/postgres${DB_QUERY}"

# Terminate existing connections; DROP DATABASE fails while any remain.
psql "${MAINT_URL}" -v ON_ERROR_STOP=1 -c \
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${DB_NAME}' AND pid <> pg_backend_pid();"

# Drop and recreate. No `|| true`: a failure here must abort the restore.
psql "${MAINT_URL}" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS \"${DB_NAME}\""
psql "${MAINT_URL}" -v ON_ERROR_STOP=1 -c "CREATE DATABASE \"${DB_NAME}\""

# Restore database (non-zero exit propagates; success is only printed after).
echo "[$(date)] Restoring database..."
pg_restore -d "${DB_URL}" --no-owner --no-acl --clean --if-exists \
    "${BACKUP_DIR}/apexintel.pgdump"

# Restore config
if [[ -d "${BACKUP_DIR}/config" ]]; then
    echo "[$(date)] Restoring config..."
    cp -r "${BACKUP_DIR}/config/"* /opt/apexintel/config/
fi

echo "[$(date)] Restore complete."
echo "[$(date)] Restart services: systemctl restart apexintel-api apexintel-worker"
