#!/usr/bin/env bash
# scripts/restore.sh — Restore ApexIntel from backup
# Usage: ./scripts/restore.sh /path/to/backup/directory

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

# Terminate existing connections
psql "${DB_URL}" -c \
    "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'apexintel' AND pid <> pg_backend_pid();" \
    2>/dev/null || true

# Drop and recreate using the full DATABASE_URL for host/port awareness
psql "${DB_URL}" -c "DROP DATABASE IF EXISTS apexintel" 2>/dev/null || true
psql "${DB_URL}" -c "CREATE DATABASE apexintel" 2>/dev/null || true

# Restore
echo "[$(date)] Restoring database..."
pg_restore -d "${DB_URL}" --no-owner --no-acl --clean --if-exists \
    "${BACKUP_DIR}/apexintel.pgdump" 2>&1 || true

# Restore config
if [[ -d "${BACKUP_DIR}/config" ]]; then
    echo "[$(date)] Restoring config..."
    cp -r "${BACKUP_DIR}/config/"* /opt/apexintel/config/ 2>/dev/null || true
fi

echo "[$(date)] Restore complete."
echo "[$(date)] Restart services: systemctl restart apexintel-api apexintel-worker apexintel-frontend"
