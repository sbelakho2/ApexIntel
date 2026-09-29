#!/usr/bin/env bash
# Sensitive-defaults guard (audit P0 #22, extended per audit P2-3).
#
# Authoritative persistence must never convert a storage failure into neutral
# data. This gate fails when production code in the audited surfaces suppresses
# a database error with a default:
#
#   .await.unwrap_or(...)          .await.unwrap_or_default()
#   .await.ok()                    .ok().flatten()
#   .await.expect(...)             .await.unwrap()
#   row.try_get(...).unwrap_or(...)
#   <line>.await
#   .unwrap_or(...)                (immediate continuation of an await)
#
# Scanned production surfaces:
#   crates/api/src/web/**
#   crates/api/src/api_handlers/**
#   crates/worker/src/**
#   crates/store/src/postgres/**
#   crates/triage/src/**
#
# Test code is exempt: a file is scanned only up to its first `#[cfg(test)]`
# attribute, which is where test modules live in this workspace.
#
# Non-database awaits (HTTP, environment, clocks, locks) may be listed in
# scripts/ci/sensitive_defaults_allowlist.txt with a reason.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

SCOPE=(
  "crates/api/src/web"
  "crates/api/src/api_handlers"
  "crates/worker/src"
  "crates/store/src/postgres"
  "crates/triage/src"
)

ALLOWLIST="scripts/ci/sensitive_defaults_allowlist.txt"

SINGLE_LINE_PATTERN='\.await[[:space:]]*\.unwrap_or|\.await[[:space:]]*\.ok\(\)|\.ok\(\)\.flatten\(\)|\.await[[:space:]]*\.expect\(|\.await[[:space:]]*\.unwrap\(\)|try_get[^;]*\.unwrap_or|try_get[^;]*\.ok\(\)\?|map_while\(|flat_map\(Result::ok\)|flat_map\(std::result::Result::ok\)'

if [ ! -f "$ALLOWLIST" ]; then
  echo "ERROR: allowlist missing: $ALLOWLIST" >&2
  exit 2
fi

is_allowed() {
  local key="$1" file="$2"
  local entry reason raw
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

# The production-code line limit per file (first #[cfg(test)] attribute).
file_cutoff() {
  local file="$1" cutoff
  cutoff="$(grep -nE '^[[:space:]]*#\[cfg\(test\)\]' "$file" | head -1 | cut -d: -f1)"
  if [ -z "$cutoff" ]; then
    wc -l < "$file"
  else
    printf '%s' "$cutoff"
  fi
}

# Test-only files (dedicated test modules / fixtures) are exempt by name.
files="$(find "${SCOPE[@]}" -name '*.rs' -type f 2>/dev/null \
  ! -name 'tests.rs' ! -name 'test_*.rs' ! -name '*_test.rs' \
  ! -path '*/tests/*' | sort)"

# ── Pass 1: single-line shapes (including row.try_get defaults). ────────────
while IFS= read -r file; do
  [ -z "$file" ] && continue
  cutoff="$(file_cutoff "$file")"
  head -n "$cutoff" "$file" | grep -nE "$SINGLE_LINE_PATTERN" | while IFS= read -r match; do
    line="${match%%:*}"
    text="${match#*:}"
    if ! is_allowed "$file:$line" "$file"; then
      echo "SENSITIVE-DEFAULT: $file:$line:$text"
      echo "__VIOLATION__"
    fi
  done
done <<< "$files" > /tmp/sensitive-defaults-pass1.out || true
violations=0
if [ -s /tmp/sensitive-defaults-pass1.out ]; then
  violations=$((violations + $(grep -c '__VIOLATION__' /tmp/sensitive-defaults-pass1.out || true)))
  grep -v '__VIOLATION__' /tmp/sensitive-defaults-pass1.out || true
fi

# ── Pass 2: `.await` at end of a line followed by a method-chain default. ───
while IFS= read -r file; do
  [ -z "$file" ] && continue
  cutoff="$(file_cutoff "$file")"
  found="$(head -n "$cutoff" "$file" | awk '
    /\.await[[:space:]]*$/ { pending = 1; next }
    pending {
      if ($0 ~ /^[[:space:]]*\.[A-Za-z_]/) {
        if ($0 ~ /\.(unwrap_or|unwrap_or_default|unwrap_or_else|ok\(\)|expect\(|unwrap\()/) {
          printf "%d:%s\n", FNR, $0
        }
        next
      }
      pending = 0
    }')"
  [ -z "$found" ] && continue
  while IFS= read -r match; do
    line="${match%%:*}"
    text="${match#*:}"
    if ! is_allowed "$file:$line" "$file"; then
      echo "SENSITIVE-DEFAULT: $file:$line:$text"
      violations=$((violations + 1))
    fi
  done <<< "$found"
done <<< "$files"

# ── Pass 3: `filter_map` whose body decodes a row with try_get. ─────────────
while IFS= read -r file; do
  [ -z "$file" ] && continue
  cutoff="$(file_cutoff "$file")"
  found="$(head -n "$cutoff" "$file" | grep -n 'filter_map(' -A6 | grep -B1 -E 'try_get\([^)]*\)\.ok\(\)' | grep 'filter_map(' || true)"
  [ -z "$found" ] && continue
  while IFS= read -r match; do
    [ -z "$match" ] && continue
    line="${match%%:*}"
    text="${match#*:}"
    if ! is_allowed "$file:$line" "$file"; then
      echo "SENSITIVE-DEFAULT: $file:$line:$text"
      violations=$((violations + 1))
    fi
  done <<< "$found"
done <<< "$files"

rm -f /tmp/sensitive-defaults-pass1.out

if [ "$violations" -gt 0 ]; then
  echo ""
  echo "FAIL: $violations sensitive-path default violation(s)."
  echo "Fix the call to propagate the error / record a degraded state, or add a"
  echo "reasoned exception to $ALLOWLIST."
  exit 1
fi

echo "PASS: sensitive-defaults guard — no storage error can be silently defaulted in the audited paths"
exit 0
