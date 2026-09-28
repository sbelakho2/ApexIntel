#!/usr/bin/env bash
# Single-frontend guard.
#
# Production ships ONE web surface: the server-rendered Askama + HTMX UI in
# `crates/api`. The Leptos/WASM SPA was retired from the workspace and archived
# under `experiments/wasm-frontend/` — it is not built, not tested, and not
# shipped. See `experiments/README.md`.
#
# Fails when the retired surface is wired back into the product:
#   1. `crates/frontend/` exists again or is a workspace member.
#   2. Any tracked file outside the archive references `crates/frontend` or the
#      `apex-frontend` package.
#   3. A wasm32/trunk build step is wired into CI, Docker, or package scripts.
#
# Run from anywhere; operates on `git ls-files` / `git grep` at the repo root.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

SELF="scripts/ci/check_frontend_removed.sh"
failed=0

fail() {
  echo "SINGLE-FRONTEND: $*" >&2
  failed=1
}

# ── 1. The crate directory and workspace membership ───────────────────────
if [ -d crates/frontend ]; then
  fail "crates/frontend/ exists again; production UI is crates/api (Askama + HTMX)"
fi

if grep -Eq '"crates/frontend"' Cargo.toml; then
  fail "Cargo.toml lists crates/frontend as a workspace member"
fi

# ── 2. References outside the experiment archive ──────────────────────────
while IFS= read -r path; do
  [ -z "${path}" ] && continue
  [ "${path}" = "${SELF}" ] && continue
  fail "tracked file references the retired WASM crate: ${path}"
done < <(git grep -l -E 'crates/frontend|apex-frontend' -- . ":!experiments" ":!${SELF}" 2>/dev/null || true)

# ── 3. wasm/trunk wired into shipped build surfaces ───────────────────────
#
# Compile-only wasm proofs are allowed and required (`cargo check -p
# apex-shared --target wasm32-unknown-unknown` keeps the cross-platform
# boundary honest). Browser-artifact tooling — trunk, wasm-pack, wasm-bindgen
# bundling — is not: production ships no browser artifact.
for f in .woodpecker.yml package.json Dockerfile.api Dockerfile.worker docker-compose.yml; do
  [ -f "${f}" ] || continue
  if grep -Eqi 'trunk|wasm-pack|wasm-bindgen' "${f}"; then
    fail "${f} wires browser-artifact tooling (trunk/wasm-pack/wasm-bindgen); no browser artifact is shipped"
  fi
  if grep -q 'wasm32' "${f}"; then
    while IFS= read -r line; do
      trimmed="${line#"${line%%[![:space:]]*}"}"
      case "${trimmed}" in
        \#*) continue ;;
      esac
      case "${line}" in
        *"cargo check"*"wasm32-unknown-unknown"*|*"rustup target add wasm32-unknown-unknown"*) ;;
        *) fail "${f} wires a wasm32 build that is not a compile-only check: ${line}" ;;
      esac
    done < <(grep 'wasm32' "${f}")
  fi
done

if [ "${failed}" -ne 0 ]; then
  echo "single-frontend guard FAILED" >&2
  exit 1
fi
echo "single-frontend guard passed (no crates/frontend references; one shipped UI)"
