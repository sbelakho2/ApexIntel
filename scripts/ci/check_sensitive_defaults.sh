#!/usr/bin/env bash
# Sensitive-path default guard (audit P0 #22).
#
# Fails when a storage/database failure can be silently converted into neutral
# data in the request/job paths that must never fabricate state:
#
#   crates/api/src/web/**
#   crates/api/src/api_handlers/**
#   crates/worker/src/job_execution/**
#
# Forbidden shapes (on awaited calls):
#   x.await.unwrap_or(..)      x.await.unwrap_or_else(..)
#   x.await.unwrap_or_default() x.await.ok()
#   x.ok().flatten()            x.await.expect(..)   x.await.unwrap()
#
# The awaits may be split across lines (`x\n  .await\n  .unwrap_or(..)`), so
# the guard checks both the single-line shape and a multi-line look-ahead after
# a trailing `.await`. It also flags `row.try_get(..)` decoding followed by a
# default, which fabricates column values when decoding fails.
#
# A violation must instead propagate the error, return an explicit API error,
# or record a DataState/degraded notice.
#
# Justified non-database awaits are listed, with a reason, in
# scripts/ci/sensitive_defaults_allowlist.txt (one `path` or `path:line` per
# line, `# reason` required).
#
# Exit codes: 0 = clean, 1 = violations found, 2 = misconfiguration.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

SCOPE=(
  "crates/api/src/web"
  "crates/api/src/api_handlers"
  "crates/worker/src/job_execution"
)

ALLOWLIST="scripts/ci/sensitive_defaults_allowlist.txt"

SINGLE_LINE_PATTERN='\.await[[:space:]]*\.unwrap_or|\.await[[:space:]]*\.ok\(\)|\.ok\(\)\.flatten\(\)|\.await[[:space:]]*\.expect\(|\.await[[:space:]]*\.unwrap\(\)|try_get[^;]*\.unwrap_or'

if [ ! -f "$ALLOWLIST" ]; then
  echo "ERROR: allowlist missing: $ALLOWLIST" >&2
  exit 2
fi

is_allowed() {
  local key="$1" file="$2"
  local entry reason
  while IFS= read -r raw || [ -n "$raw" ]; do
    entry="${raw%%#*}"
    reason="${raw#"${entry}"}"
    entry="$(printf '%s' "$entry" | tr -d '[:space:]')"
    [ -z "$entry" ] && continue
    if [ -z "$(printf '%s' "$reason" | tr -d '[:space:]#')" ]; then
      echo "ERROR: allowlist entry without a reason: $raw" >&2
      exit 2
    fi
    if [ "$entry" = "$key" ] || [ "$entry" = "$file" ]; then
      return 0
    fi
  done < "$ALLOWLIST"
  return 1
}

violations=0
report() {
  local file="$1" line="$2" text="$3"
  if is_allowed "$file:$line" "$file"; then
    return 0
  fi
  echo "SENSITIVE-DEFAULT: $file:$line: $text"
  violations=$((violations + 1))
}

# ── Pass 1: single-line shapes (including row.try_get defaults). ────────────
while IFS= read -r match; do
  [ -z "$match" ] && continue
  file="${match%%:*}"
  rest="${match#*:}"
  line="${rest%%:*}"
  text="${rest#*:}"
  report "$file" "$line" "$text"
done < <(grep -rnE "$SINGLE_LINE_PATTERN" "${SCOPE[@]}" --include='*.rs' || true)

# ── Pass 2: `.await` at end of a line followed by a method-chain default. ───
# Only immediate continuation lines (starting with `.`) are followed, so a
# default on an unrelated struct field a few lines later is not a violation.
while IFS= read -r match; do
  [ -z "$match" ] && continue
  file="${match%%:*}"
  rest="${match#*:}"
  line="${rest%%:*}"
  text="${rest#*:}"
  report "$file" "$line" "$text"
done < <(
  awk '
    /\.await[[:space:]]*$/ { pending = 1; next }
    pending {
      if ($0 ~ /^[[:space:]]*\.[A-Za-z_]/) {
        if ($0 ~ /\.(unwrap_or|unwrap_or_default|unwrap_or_else|ok\(\)|expect\(|unwrap\()/) {
          printf "%s:%d:%s\n", FILENAME, FNR, $0
        }
        next
      }
      pending = 0
    }
  ' $(find "${SCOPE[@]}" -name '*.rs' -type f)
)

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "FAIL: $violations sensitive-path default violation(s)."
  echo "Fix the call to propagate the error / record a degraded state, or add a"
  echo "reasoned exception to $ALLOWLIST."
  exit 1
fi

echo "PASS: sensitive-defaults guard — no storage error can be silently defaulted in the audited paths"
exit 0
