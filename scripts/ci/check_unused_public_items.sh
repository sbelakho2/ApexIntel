#!/usr/bin/env bash
# #166: fail when a workspace crate exposes a `pub` item that is reachable only
# from inside that crate (unused or effectively private public API).
#
# `unreachable_pub` is a stable rustc lint that reports `pub` items declared in
# private modules, or in public modules that are themselves unreachable, i.e.
# API that nothing outside the crate can name. Cargo passes `--cap-lints allow`
# to external dependencies, so the lint only applies to workspace members.
#
# The check uses `-W` rather than `-D`: a hard deny stops cargo at the first
# failing crate, which hides every finding in crates that depend on it. Warning
# level lets the whole workspace compile so this gate reports the COMPLETE
# finding list in one run, and then the script fails when any finding exists.
#
# `--all-targets` is intentional: `pub` items reachable only from a crate's own
# tests/benches are not real published API and must not pass the gate.
#
# Run locally with: bash scripts/ci/check_unused_public_items.sh
# npm alias:        npm run check:unused-public
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

OUTPUT="$(mktemp "${TMPDIR:-/tmp}/apex-unused-public.XXXXXX")"
trap 'rm -f "$OUTPUT"' EXIT

# Preserve caller-provided flags (CI sets debuginfo/linker flags for the large
# worker test binaries) and append the lint.
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-W unreachable_pub"

cargo check --workspace --all-targets --locked 2>&1 | tee "$OUTPUT"

FINDINGS="$(grep -c 'unreachable `pub` item' "$OUTPUT" || true)"
FINDINGS="${FINDINGS:-0}"

if [ "$FINDINGS" -gt 0 ]; then
  printf '\n%s\n' \
    "check_unused_public_items: ${FINDINGS} unreachable public item(s) found (#166)."
  printf '%s\n' \
    "Restrict each item to pub(crate)/pub(super), or export it for real use from a public module."
  exit 1
fi

printf '%s\n' "check_unused_public_items: no unreachable public items found."
