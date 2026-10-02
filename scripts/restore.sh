#!/usr/bin/env bash
# scripts/restore.sh — Restore ApexIntel from backup
# Usage: ./scripts/restore.sh /path/to/backup/directory
#
# The destructive steps below run under `set -e`: a failed terminate/drop/
# create/restore aborts the script with a non-zero exit instead of falling
# through to an unconditional "Restore complete."

#
# Encrypted backups (apexintel.pgdump.age, see scripts/backup.sh) need
# BACKUP_AGE_IDENTITY set to the age identity file; they are decrypted in a
# stream so the plaintext dump never touches disk.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/backup_common.sh
source "${SCRIPT_DIR}/lib/backup_common.sh"

BACKUP_DIR="${1:?Usage: restore.sh /path/to/backup/dir}"

: "${DATABASE_URL:?Set DATABASE_URL before running scripts/restore.sh}"

resolve_dump_path "${BACKUP_DIR}"
if [[ "${DUMP_PATH}" == *.age ]]; then
    # Fail before anything is dropped, not halfway through the restore: a
    # full authenticated decrypt proves the key matches and the file is intact.
    : "${BACKUP_AGE_IDENTITY:?Set BACKUP_AGE_IDENTITY to decrypt ${DUMP_PATH}}"
    command -v age >/dev/null || { echo "ERROR: age is not installed" >&2; exit 1; }
    stream_dump > /dev/null
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

pg_url_split "${DB_URL}"
DB_NAME="${PG_DB_NAME}"
MAINT_URL="${PG_MAINT_URL}"

# Terminate existing connections; DROP DATABASE fails while any remain.
psql "${MAINT_URL}" -v ON_ERROR_STOP=1 -c \
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${DB_NAME}' AND pid <> pg_backend_pid();"

# Drop and recreate. No `|| true`: a failure here must abort the restore.
psql "${MAINT_URL}" -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS \"${DB_NAME}\""
psql "${MAINT_URL}" -v ON_ERROR_STOP=1 -c "CREATE DATABASE \"${DB_NAME}\""

# Restore database (non-zero exit propagates; success is only printed after).
echo "[$(date)] Restoring database..."
stream_dump | pg_restore -d "${DB_URL}" --no-owner --no-acl --clean --if-exists

# Restore config
if [[ -d "${BACKUP_DIR}/config" ]]; then
    echo "[$(date)] Restoring config..."
    cp -r "${BACKUP_DIR}/config/"* /opt/apexintel/config/
fi

echo "[$(date)] Restore complete."
echo "[$(date)] Restart services: systemctl restart apexintel-api apexintel-worker"
