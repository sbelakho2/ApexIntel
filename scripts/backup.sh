#!/usr/bin/env bash
# scripts/backup.sh — ApexIntel database + config backup
# Usage: ./scripts/backup.sh [destination_dir]
# Cron:  0 4 * * * /opt/apexintel/scripts/backup.sh /opt/apexintel/backups
#
# Encryption: set BACKUP_AGE_RECIPIENTS_FILE to an age recipients file (public
# keys, one per line) and the data dump is written as apexintel.pgdump.age;
# the plaintext dump never outlives the run. Restore with BACKUP_AGE_IDENTITY
# pointing at the matching private key (scripts/restore.sh,
# scripts/restore_validate.sh). Without it the dump stays plaintext and a
# warning is logged on every run. Every artefact is owner-only (umask 077).
#
# Exits non-zero when the dump, the integrity check or the encryption fails,
# so the systemd unit and its alerting see a failed backup as failed.

set -euo pipefail
umask 077

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=scripts/lib/backup_common.sh
source "${SCRIPT_DIR}/lib/backup_common.sh"

DEST="${1:-/opt/apexintel/backups}"
DATE=$(date +%Y%m%d_%H%M%S)
BACKUP_DIR="${DEST}/${DATE}"
RETENTION_DAYS=30
: "${BACKUP_AGE_RECIPIENTS_FILE:=}"

: "${DATABASE_URL:?Set DATABASE_URL before running scripts/backup.sh}"

if [[ -n "${BACKUP_AGE_RECIPIENTS_FILE}" ]]; then
    command -v age >/dev/null || { echo "ERROR: BACKUP_AGE_RECIPIENTS_FILE is set but age is not installed" >&2; exit 1; }
    [[ -r "${BACKUP_AGE_RECIPIENTS_FILE}" ]] || { echo "ERROR: cannot read ${BACKUP_AGE_RECIPIENTS_FILE}" >&2; exit 1; }
fi

mkdir -p "${DEST}" "${BACKUP_DIR}"
chmod 700 "${DEST}" "${BACKUP_DIR}" 2>/dev/null || true

# A failed run must not leave a partial (or plaintext) directory behind that
# retention and restores would treat as a good backup.
BACKUP_OK=0
on_exit() {
    if [[ "${BACKUP_OK}" -ne 1 ]]; then
        echo "[$(date)] ERROR: backup failed; removing ${BACKUP_DIR}" >&2
        rm -rf "${BACKUP_DIR}"
    fi
}
trap on_exit EXIT

echo "[$(date)] Starting ApexIntel backup to ${BACKUP_DIR}"

# 1. PostgreSQL full dump (custom format for parallel restore)
echo "[$(date)] Dumping PostgreSQL..."
pg_dump -Fc -Z 6 -f "${BACKUP_DIR}/apexintel.pgdump" "${DATABASE_URL}"

# 2. Verify the dump is a readable archive before it is trusted or encrypted.
echo "[$(date)] Verifying backup..."
if ! pg_restore --list "${BACKUP_DIR}/apexintel.pgdump" > /dev/null; then
    echo "[$(date)] ERROR: backup verification failed" >&2
    exit 1
fi
echo "[$(date)] Backup verification: OK"

# 3. Encrypt the data dump.
if [[ -n "${BACKUP_AGE_RECIPIENTS_FILE}" ]]; then
    echo "[$(date)] Encrypting dump with age..."
    age --encrypt --recipients-file "${BACKUP_AGE_RECIPIENTS_FILE}" \
        --output "${BACKUP_DIR}/apexintel.pgdump.age" "${BACKUP_DIR}/apexintel.pgdump"
    rm -f "${BACKUP_DIR}/apexintel.pgdump"
else
    echo "[$(date)] WARNING: BACKUP_AGE_RECIPIENTS_FILE is unset; the data dump is stored unencrypted" >&2
fi

# 4. Schema-only dump (human-readable, for reference; carries no row data)
echo "[$(date)] Dumping schema..."
pg_dump --schema-only -f "${BACKUP_DIR}/schema.sql" "${DATABASE_URL}"

# 5. Config files (never the raw env file: a backup must not carry live
# credentials; step 7 writes a keys-only redacted view instead)
echo "[$(date)] Backing up config..."
if [[ -d /opt/apexintel/config ]]; then
    mkdir -p "${BACKUP_DIR}/config"
    # Best-effort: an unreadable file (e.g. a root-owned certificate) must not
    # fail the database backup, but it must not go unnoticed either.
    if ! (cd /opt/apexintel/config && tar -cf - --exclude='.env' --exclude='*.env' .) \
        | (cd "${BACKUP_DIR}/config" && tar -xf -); then
        echo "[$(date)] WARNING: config copy was incomplete" >&2
    fi
fi

# 6. Recipe seed (in case of local modifications)
if [[ -f /opt/apexintel/config/recipes_seed.yaml ]]; then
    cp /opt/apexintel/config/recipes_seed.yaml "${BACKUP_DIR}/"
fi

# 7. Environment file (secrets redacted)
if [[ -f /opt/apexintel/config/.env ]]; then
    sed 's/=.*/=REDACTED/' /opt/apexintel/config/.env > "${BACKUP_DIR}/env_keys.txt"
fi

BACKUP_OK=1

# 8. Compute sizes
TOTAL_SIZE=$(du -sh "${BACKUP_DIR}" | cut -f1)
echo "[$(date)] Backup complete: ${TOTAL_SIZE} in ${BACKUP_DIR}"

# 9. Cleanup old backups: only timestamped run directories, never DEST itself
# or unrelated files that happen to live beside them.
echo "[$(date)] Cleaning backups older than ${RETENTION_DAYS} days..."
find "${DEST}" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*_[0-9]*' \
    -mtime +"${RETENTION_DAYS}" -exec rm -rf {} +

REMAINING=$(find "${DEST}" -mindepth 1 -maxdepth 1 -type d -name '[0-9]*_[0-9]*' | wc -l | tr -d ' ')
echo "[$(date)] Backup retention: ${REMAINING} backups kept"

echo "[$(date)] Done."
