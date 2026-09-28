#!/usr/bin/env bash
set -euo pipefail

# Docs contract guard (audit item 4):
#   1. Every DEPLOYMENT.md line building apex-api/apex-worker carries
#      `--features llm --locked`.
#   2. DEPLOYMENT.md contains no raw IPv4 address other than 127.0.0.1.
#   3. DEPLOYMENT.md documents the two-role DB contract and the
#      legacy-password-hash kill switch.
#   4. DEPLOYMENT.md contains none of the banned production identifiers.

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DEPLOYMENT_MD="$REPO_ROOT/DEPLOYMENT.md"

if [[ ! -f "$DEPLOYMENT_MD" ]]; then
    echo "FAIL: docs-contract — $DEPLOYMENT_MD not found"
    exit 1
fi

status=0

fail() {
    echo "FAIL: docs-contract — $1"
    status=1
}

# ── 1. Build commands for both binaries must use --features llm --locked ─────
while IFS= read -r match; do
    [[ -z "$match" ]] && continue
    lineno="${match%%:*}"
    text="${match#*:}"
    if [[ "$text" != *"--features llm"* ]] || [[ "$text" != *"--locked"* ]]; then
        fail "build command missing '--features llm --locked' at DEPLOYMENT.md:$lineno"
        echo "      $text"
    fi
done < <(grep -nE 'cargo (zig)?build .*-p (apex-api|apex-worker)' "$DEPLOYMENT_MD" || true)

# ── 2. No raw IPv4 addresses except 127.0.0.1 ────────────────────────────────
IPV4_RE='[0-9]{1,3}(\.[0-9]{1,3}){3}'
while IFS= read -r match; do
    [[ -z "$match" ]] && continue
    lineno="${match%%:*}"
    text="${match#*:}"
    while IFS= read -r ip; do
        [[ -z "$ip" ]] && continue
        [[ "$ip" == "127.0.0.1" ]] && continue
        fail "raw IPv4 address '$ip' at DEPLOYMENT.md:$lineno"
        echo "      $text"
    done < <(printf '%s\n' "$text" | grep -oE "$IPV4_RE" || true)
done < <(grep -nE "$IPV4_RE" "$DEPLOYMENT_MD" || true)

# ── 3. Required contract strings ─────────────────────────────────────────────
for needle in \
    "MIGRATION_DATABASE_URL" \
    "apexintel_migrator" \
    "apexintel_app" \
    "ALLOW_LEGACY_PASSWORD_HASHES=false"
do
    if ! grep -qF -- "$needle" "$DEPLOYMENT_MD"; then
        fail "missing required string '$needle' in DEPLOYMENT.md"
    fi
done

# ── 4. Banned production identifiers ─────────────────────────────────────────
for banned in \
    "starzerp.fi" \
    "77.42.65.89" \
    "198.53.64.194" \
    "hetzner-db-mac" \
    "vastai_new"
do
    matches="$(grep -nF -- "$banned" "$DEPLOYMENT_MD" || true)"
    if [[ -n "$matches" ]]; then
        fail "banned string '$banned' present in DEPLOYMENT.md"
        while IFS= read -r line; do
            [[ -z "$line" ]] && continue
            echo "      $line"
        done <<< "$matches"
    fi
done

if [[ "$status" -eq 0 ]]; then
    echo "PASS: docs-contract — DEPLOYMENT.md build flags, no raw IPs, DB role contract, and identifier redaction are OK"
    exit 0
fi

echo "FAIL: docs-contract — see offending lines above"
exit 1
