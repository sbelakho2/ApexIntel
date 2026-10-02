# shellcheck shell=bash
# scripts/lib/backup_common.sh — helpers shared by backup.sh, restore.sh and
# restore_validate.sh. Source it; it defines functions only.

# Split a postgres URL into PG_DB_NAME and PG_MAINT_URL (same server, the
# `postgres` maintenance database, query string preserved). DROP/CREATE
# DATABASE cannot run through a connection to the database being replaced.
pg_url_split() {
    local url="$1"
    local no_query="${url%%\?*}"
    local query=""
    if [[ "${url}" == *\?* ]]; then
        query="?${url#*\?}"
    fi
    PG_DB_NAME="${no_query##*/}"
    if [[ -z "${PG_DB_NAME}" || "${PG_DB_NAME}" == "${no_query}" ]]; then
        echo "ERROR: cannot derive a database name from ${url%%@*}@…" >&2
        return 1
    fi
    if [[ ! "${PG_DB_NAME}" =~ ^[A-Za-z0-9_-]+$ ]]; then
        echo "ERROR: unsupported database name '${PG_DB_NAME}'" >&2
        return 1
    fi
    # Output for the sourcing script (restore.sh / restore_validate.sh).
    # shellcheck disable=SC2034
    PG_MAINT_URL="${no_query%/*}/postgres${query}"
    PG_SERVER="${no_query%/*}"
    PG_SERVER="${PG_SERVER##*@}"
}

# Locate the dump inside a backup directory (or accept a dump path directly)
# and set DUMP_PATH. Encrypted dumps (`.age`) win over plaintext ones.
resolve_dump_path() {
    local path="$1"
    if [[ -d "${path}" ]]; then
        if [[ -f "${path}/apexintel.pgdump.age" ]]; then
            DUMP_PATH="${path}/apexintel.pgdump.age"
        else
            DUMP_PATH="${path}/apexintel.pgdump"
        fi
    else
        DUMP_PATH="${path}"
    fi
    if [[ ! -f "${DUMP_PATH}" ]]; then
        echo "ERROR: ${DUMP_PATH} not found" >&2
        return 1
    fi
}

# Stream the custom-format dump at DUMP_PATH to stdout, decrypting `.age`
# dumps with the identity file in BACKUP_AGE_IDENTITY. Plaintext never
# touches disk.
stream_dump() {
    if [[ "${DUMP_PATH}" == *.age ]]; then
        : "${BACKUP_AGE_IDENTITY:?Set BACKUP_AGE_IDENTITY to the age identity file that decrypts ${DUMP_PATH}}"
        command -v age >/dev/null || { echo "ERROR: age is not installed" >&2; return 1; }
        age --decrypt --identity "${BACKUP_AGE_IDENTITY}" "${DUMP_PATH}"
    else
        cat "${DUMP_PATH}"
    fi
}
