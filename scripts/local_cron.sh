#!/usr/bin/env bash
# Local cron jobs for ApexIntel development environment
# Usage: source this from crontab, or run individual functions

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

HEALTH_URL="http://localhost:9095/api/health"
BACKUP_DIR="${REPO_ROOT}/backups"
DB_NAME="apexintel"
LOG_FILE="${BACKUP_DIR}/cron.log"

# 1. Health check (every 6 hours)
health_check() {
    echo "[$(date)] Health check..." >> "$LOG_FILE"
    curl -s "$HEALTH_URL" >> "$LOG_FILE" 2>&1
    echo "" >> "$LOG_FILE"
}

# 2. Database backup (daily at 3 AM local time)
db_backup() {
    echo "[$(date)] Starting DB backup..." >> "$LOG_FILE"
    # Dumps hold the full dataset: keep them owner-only.
    local dump
    dump="${BACKUP_DIR}/apexintel_$(date +%Y%m%d).pgdump"
    (umask 077 && pg_dump -Fc -Z 6 -f "$dump" "$DB_NAME" 2>> "$LOG_FILE")
    echo "[$(date)] Backup complete: $(du -sh "$dump" | cut -f1)" >> "$LOG_FILE"
}

# 3. Backup cleanup (daily at 3:07 AM, keep 30 days)
backup_cleanup() {
    echo "[$(date)] Cleaning old backups..." >> "$LOG_FILE"
    find "$BACKUP_DIR" -name "apexintel_*.pgdump" -mtime +30 -delete 2>> "$LOG_FILE"
    echo "[$(date)] Cleanup done. Remaining: $(find "$BACKUP_DIR" -maxdepth 1 -name 'apexintel_*.pgdump' | wc -l)" >> "$LOG_FILE"
}

# 4. Worker alive check (every 10 minutes)
worker_check() {
    if ! pgrep -f "apex-worker" > /dev/null; then
        echo "[$(date)] WARNING: Worker not running! Restarting..." >> "$LOG_FILE"
        cd "${REPO_ROOT}" || { echo "[$(date)] ERROR: cannot cd to ${REPO_ROOT}" >> "$LOG_FILE"; return 1; }
        nohup ./target/release/apex-worker > /tmp/apex-worker.log 2>&1 &
        echo "[$(date)] Worker restarted (PID: $!)" >> "$LOG_FILE"
    fi
}

# 5. API alive check (every 10 minutes)
api_check() {
    if ! pgrep -f "apex-api" > /dev/null; then
        echo "[$(date)] WARNING: API not running! Restarting..." >> "$LOG_FILE"
        cd "${REPO_ROOT}" || { echo "[$(date)] ERROR: cannot cd to ${REPO_ROOT}" >> "$LOG_FILE"; return 1; }
        nohup ./target/release/apex-api > /tmp/apex-api.log 2>&1 &
        echo "[$(date)] API restarted (PID: $!)" >> "$LOG_FILE"
    fi
}

# Dispatch based on first argument
case "${1:-}" in
    health) health_check ;;
    backup) db_backup ;;
    cleanup) backup_cleanup ;;
    worker) worker_check ;;
    api) api_check ;;
    *) echo "Usage: $0 {health|backup|cleanup|worker|api}" ;;
esac
