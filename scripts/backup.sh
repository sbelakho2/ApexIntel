#!/usr/bin/env bash
# scripts/backup.sh — ApexIntel database + config backup
# Usage: ./scripts/backup.sh [destination_dir]
# Cron:  0 4 * * * /opt/apexintel/scripts/backup.sh /opt/apexintel/backups

set -euo pipefail

DEST="${1:-/opt/apexintel/backups}"
DATE=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR="${DEST}/${DATE}"
RETENTION_DAYS=30

: "${DATABASE_URL:?Set DATABASE_URL before running scripts/backup.sh}"

mkdir -p "${BACKUP_DIR}"

echo "[$(date)] Starting ApexIntel backup to ${BACKUP_DIR}"

# 1. PostgreSQL full dump (custom format for parallel restore)
echo "[$(date)] Dumping PostgreSQL..."
pg_dump -Fc -Z 6 -f "${BACKUP_DIR}/apexintel.pgdump" \
    "${DATABASE_URL}" 2>&1

# 2. Schema-only dump (human-readable, for reference)
echo "[$(date)] Dumping schema..."
pg_dump --schema-only -f "${BACKUP_DIR}/schema.sql" \
    "${DATABASE_URL}" 2>&1

# 3. Config files
echo "[$(date)] Backing up config..."
cp -r /opt/apexintel/config/ "${BACKUP_DIR}/config/" 2>/dev/null || true

# 4. Recipe seed (in case of local modifications)
cp /opt/apexintel/config/recipes_seed.yaml "${BACKUP_DIR}/" 2>/dev/null || true

# 5. Environment file (secrets redacted)
if [[ -f /opt/apexintel/config/.env ]]; then
    sed 's/=.*/=REDACTED/' /opt/apexintel/config/.env > "${BACKUP_DIR}/env_keys.txt"
fi

# 6. Compute sizes
TOTAL_SIZE=$(du -sh "${BACKUP_DIR}" | cut -f1)
echo "[$(date)] Backup complete: ${TOTAL_SIZE} in ${BACKUP_DIR}"

# 7. Cleanup old backups
echo "[$(date)] Cleaning backups older than ${RETENTION_DAYS} days..."
find "${DEST}" -maxdepth 1 -type d -mtime +${RETENTION_DAYS} -exec rm -rf {} + 2>/dev/null || true

REMAINING=$(ls -1d "${DEST}"/*/ 2>/dev/null | wc -l)
echo "[$(date)] Backup retention: ${REMAINING} backups kept"

# 8. Verify backup integrity
echo "[$(date)] Verifying backup..."
pg_restore --list "${BACKUP_DIR}/apexintel.pgdump" > /dev/null 2>&1 && \
    echo "[$(date)] Backup verification: OK" || \
    echo "[$(date)] WARNING: Backup verification failed!"

echo "[$(date)] Done."
