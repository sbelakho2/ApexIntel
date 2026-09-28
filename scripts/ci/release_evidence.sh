#!/usr/bin/env bash
# Release evidence bundle (P1-8) — the exact-SHA release contract recorder.
#
# A release is only evidenced when every gate below passed for the exact commit
# being released, and the bundle records that commit, the per-gate results, the
# artifact digests and the timestamps:
#
#   exact-sha            HEAD == --sha and the git worktree is clean
#   rustfmt              cargo fmt --all -- --check
#   clippy-default       cargo clippy --workspace --all-targets --locked -- -D warnings
#   clippy-all-features  cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
#   unit-tests           cargo test --workspace --locked --no-fail-fast
#   all-features-tests   cargo test --workspace --all-features --locked --no-fail-fast
#   pg-canonical         scripts/ci/run_pg_integration_suites.sh (needs TEST_DATABASE_URL)
#   migration-bootstrap  fresh-database migration chain (apex-store migrations_integration)
#   browser-integration  scripts/ci/e2e_server_ui.mjs route sweep (needs a running API)
#   ui-journey           npm run test:server-ui:journeys (needs a running API)
#   tailwind-assets      rebuild the committed stylesheet
#   wasm-shared          cargo check -p apex-shared --target wasm32-unknown-unknown --locked
#   container-browser    scripts/ci/browser_container_check.sh (needs a Docker daemon)
#
# Gates owned by dedicated pipeline steps can be recorded rather than re-run,
# because the evidence step is only reached after the owning steps succeeded:
#   --record <gate>=passed|failed    source is recorded as "pipeline-step"
# Gates that were neither executed nor recorded are published as `not_run`, so
# a bundle never claims a gate that did not happen (complete=false).
#
# Usage:
#   scripts/ci/release_evidence.sh [--sha SHA] [--out DIR]
#                                  [--gate NAME]... [--record NAME=passed|failed]...
#                                  [--artifact PATH] [--artifact-digest DIGEST]
#                                  [--list-gates] [--self-test]
#
# Output (default dir release-evidence/<sha>/):
#   release-evidence.json   machine-readable bundle
#   release-evidence.txt    human-readable summary
#   release-evidence.sha256 sha256 of the two bundle files
#   logs/<gate>.log         captured output for executed gates
#   tailwind.css            rebuilt stylesheet from the tailwind-assets gate
#
# Environment (first non-empty wins; keep in sync with
# scripts/ops/record_deployment.sh and crates/api/src/provenance.rs):
#   APEX_GIT_SHA / GIT_SHA / CI_COMMIT_SHA       commit to evidence
#   APEX_CI_PIPELINE_ID / CI_PIPELINE_ID / CI_PIPELINE_NUMBER
#   APEX_BUILD_TIMESTAMP / BUILD_TIMESTAMP / CI_PIPELINE_CREATED
#   APEX_ARTIFACT_DIGEST / ARTIFACT_DIGEST
#   APEX_EVIDENCE_DIR                            default --out
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

# Container steps run as root while the agent checks the workspace out as the
# host user; without this, every git call fails with "dubious ownership" and
# git-based checks silently see an empty repository.
git config --global --add safe.directory '*' 2>/dev/null || true


SCHEMA_VERSION=1
CANONICAL_GATES="exact-sha rustfmt clippy-default clippy-all-features unit-tests all-features-tests pg-canonical migration-bootstrap browser-integration ui-journey tailwind-assets wasm-shared container-browser"

sha=""
out_dir=""
artifact_path=""
artifact_digest=""
self_test=0
list_gates=0
selected_gates=""
record_names=()
record_statuses=()

usage() {
  sed -n '2,58p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --sha) sha="$2"; shift 2 ;;
    --out) out_dir="$2"; shift 2 ;;
    --gate) selected_gates="${selected_gates} $2"; shift 2 ;;
    --record)
      record="${2:-}"
      record_name="${record%%=*}"
      record_status="${record#*=}"
      case "${record_status}" in
        passed|failed) ;;
        *)
          echo "invalid --record '${record}' (expected <gate>=passed|failed)" >&2
          exit 2
          ;;
      esac
      if [ -z "${record_name}" ] || [ "${record_name}" = "${record}" ]; then
        echo "invalid --record '${record}' (expected <gate>=passed|failed)" >&2
        exit 2
      fi
      record_names[${#record_names[@]}]="${record_name}"
      record_statuses[${#record_statuses[@]}]="${record_status}"
      shift 2
      ;;
    --artifact) artifact_path="$2"; shift 2 ;;
    --artifact-digest) artifact_digest="$2"; shift 2 ;;
    --list-gates) list_gates=1; shift ;;
    --self-test) self_test=1; shift ;;
    -h|--help) usage 0 ;;
    *) echo "unknown argument: $1" >&2; usage 2 ;;
  esac
done

if [ "${list_gates}" -eq 1 ]; then
  for gate in ${CANONICAL_GATES}; do
    echo "${gate}"
  done
  exit 0
fi

in_list() { # <needle> <space-separated list>
  local needle="$1" item
  for item in $2; do
    [ "${item}" = "${needle}" ] && return 0
  done
  return 1
}

for gate in ${selected_gates}; do
  if ! in_list "${gate}" "${CANONICAL_GATES}"; then
    echo "unknown --gate '${gate}' (see --list-gates)" >&2
    exit 2
  fi
done
index=0
while [ "${index}" -lt "${#record_names[@]}" ]; do
  if ! in_list "${record_names[${index}]}" "${CANONICAL_GATES}"; then
    echo "unknown --record gate '${record_names[${index}]}' (see --list-gates)" >&2
    exit 2
  fi
  index=$((index + 1))
done

recorded_status() { # <gate> -> prints passed/failed or nothing
  local gate="$1" index=0
  while [ "${index}" -lt "${#record_names[@]}" ]; do
    if [ "${record_names[${index}]}" = "${gate}" ]; then
      echo "${record_statuses[${index}]}"
      return 0
    fi
    index=$((index + 1))
  done
  return 1
}

json_escape() {
  # Values are single-line JSON strings: collapse control characters and
  # escape backslashes/quotes, so an env-provided field can never produce an
  # unparsable evidence bundle.
  printf '%s' "$1" | tr '\n\r\t' ' ' | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

digest_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    printf 'sha256:%s' "$(sha256sum "$1" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    printf 'sha256:%s' "$(shasum -a 256 "$1" | awk '{print $1}')"
  else
    return 1
  fi
}

# ─── Resolve the release identity ────────────────────────────────────────────
sha="${sha:-${APEX_GIT_SHA:-${GIT_SHA:-${CI_COMMIT_SHA:-}}}}"
if [ -z "${sha}" ]; then
  sha="$(git rev-parse HEAD 2>/dev/null || true)"
fi
if [ -z "${sha}" ]; then
  echo "release-evidence: cannot resolve the commit SHA (pass --sha)" >&2
  exit 2
fi
if ! printf '%s' "${sha}" | grep -Eq '^[0-9a-fA-F]{7,40}$'; then
  echo "release-evidence: invalid commit SHA '${sha}' (expected 7-40 hex characters)" >&2
  exit 2
fi
sha="$(printf '%s' "${sha}" | tr 'A-F' 'a-f')"

pipeline_id="${APEX_CI_PIPELINE_ID:-${CI_PIPELINE_ID:-${CI_PIPELINE_NUMBER:-unknown}}}"
build_timestamp="${APEX_BUILD_TIMESTAMP:-${BUILD_TIMESTAMP:-${CI_PIPELINE_CREATED:-unknown}}}"

if [ -z "${artifact_digest}" ]; then
  artifact_digest="${APEX_ARTIFACT_DIGEST:-${ARTIFACT_DIGEST:-}}"
fi
if [ -z "${artifact_digest}" ] && [ -n "${artifact_path}" ]; then
  if [ ! -f "${artifact_path}" ]; then
    echo "release-evidence: artifact not found: ${artifact_path}" >&2
    exit 2
  fi
  artifact_digest="$(digest_file "${artifact_path}")"
fi
if [ -z "${artifact_digest}" ] && [ -f "target/release/apex-api" ]; then
  artifact_digest="$(digest_file target/release/apex-api)"
fi
if [ -z "${artifact_digest}" ]; then
  artifact_digest="unknown"
fi

head_sha="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
worktree_clean=0
if [ -z "$(git status --porcelain 2>/dev/null)" ]; then
  worktree_clean=1
fi
exact_sha=false
if [ "${head_sha}" = "${sha}" ] && [ "${worktree_clean}" -eq 1 ]; then
  exact_sha=true
fi

if [ -z "${out_dir}" ]; then
  out_dir="${APEX_EVIDENCE_DIR:-${ROOT}/release-evidence/${sha}}"
fi
mkdir -p "${out_dir}/logs"

# Gate commands read the request identity from the environment instead of
# having it interpolated into the executed command string, so a crafted --sha
# or --out cannot inject shell syntax.
export APEX_EVIDENCE_SHA="${sha}"
export APEX_EVIDENCE_TAILWIND="${out_dir}/tailwind.css"

# ─── Gate commands ───────────────────────────────────────────────────────────
gate_command() { # <gate> -> the exact release command
  case "$1" in
    exact-sha)
      printf 'test "$(git rev-parse HEAD)" = "${APEX_EVIDENCE_SHA}" && test -z "$(git status --porcelain)"'
      ;;
    rustfmt) printf 'cargo fmt --all -- --check' ;;
    clippy-default) printf 'cargo clippy --workspace --all-targets --locked -- -D warnings' ;;
    clippy-all-features)
      printf 'cargo clippy --workspace --all-targets --all-features --locked -- -D warnings'
      ;;
    unit-tests) printf 'cargo test --workspace --locked --no-fail-fast' ;;
    all-features-tests) printf 'cargo test --workspace --all-features --locked --no-fail-fast' ;;
    pg-canonical) printf 'bash scripts/ci/run_pg_integration_suites.sh' ;;
    migration-bootstrap)
      printf 'cargo test -p apex-store --test migrations_integration --locked -- --ignored --test-threads=1'
      ;;
    browser-integration) printf 'node scripts/ci/e2e_server_ui.mjs' ;;
    ui-journey) printf 'npm run test:server-ui:journeys' ;;
    tailwind-assets)
      printf 'npx tailwindcss -i crates/api/static/css/globals.css -o "${APEX_EVIDENCE_TAILWIND}" --minify'
      ;;
    wasm-shared)
      printf 'cargo check -p apex-shared --target wasm32-unknown-unknown --locked'
      ;;
    container-browser) printf 'bash scripts/ci/browser_container_check.sh' ;;
    *)
      echo "release-evidence: unknown gate '${1}'" >&2
      return 1
      ;;
  esac
}

# ─── Recorder ────────────────────────────────────────────────────────────────
GATES_JSON=""
GATES_COUNT=0
FAILED=0
EXECUTED=0
RECORDED=0
NOT_RUN=0

append_gate() { # <json-entry>
  if [ "${GATES_COUNT}" -gt 0 ]; then
    GATES_JSON="${GATES_JSON},"
  fi
  GATES_JSON="${GATES_JSON}${1}"
  GATES_COUNT=$((GATES_COUNT + 1))
}

execute_gate() { # <gate>
  local gate="$1"
  local cmd log started ended exit_code status summary
  cmd="$(gate_command "${gate}")"
  log="${out_dir}/logs/${gate}.log"
  started="$(date +%s)"
  set +e
  bash -c "${cmd}" >"${log}" 2>&1
  exit_code=$?
  set -e
  ended="$(date +%s)"
  if [ "${exit_code}" -eq 0 ]; then
    status="passed"
  else
    status="failed"
    FAILED=1
  fi
  EXECUTED=$((EXECUTED + 1))
  summary="$(tail -n 5 "${log}" 2>/dev/null | tr '\n' ' ' | sed -e 's/[[:space:]]\{1,\}/ /g' -e 's/"/ /g' | cut -c1-400)"
  append_gate "$(printf '{"name":"%s","status":"%s","source":"executed","exit_code":%d,"duration_seconds":%d,"command":"%s","summary":"%s"}' \
    "$(json_escape "${gate}")" "${status}" "${exit_code}" "$((ended - started))" \
    "$(json_escape "${cmd}")" "$(json_escape "${summary}")")"
  echo "release-evidence: ${gate}: ${status} (${exit_code})"
}

record_pipeline_gate() { # <gate> <status>
  local gate="$1" status="$2"
  if [ "${status}" = "failed" ]; then
    FAILED=1
  fi
  RECORDED=$((RECORDED + 1))
  append_gate "$(printf '{"name":"%s","status":"%s","source":"pipeline-step","summary":"recorded by the owning pipeline step"}' \
    "$(json_escape "${gate}")" "${status}")"
  echo "release-evidence: ${gate}: ${status} (pipeline step)"
}

mark_not_run() { # <gate>
  NOT_RUN=$((NOT_RUN + 1))
  append_gate "$(printf '{"name":"%s","status":"not_run","source":"pending","summary":"not executed in this bundle"}' \
    "$(json_escape "${1}")")"
}

run_gate() { # <gate>
  local gate="$1" status
  if in_list "${gate}" "${selected_gates}"; then
    execute_gate "${gate}"
    return 0
  fi
  if status="$(recorded_status "${gate}")"; then
    record_pipeline_gate "${gate}" "${status}"
    return 0
  fi
  mark_not_run "${gate}"
}

if [ "${self_test}" -eq 1 ]; then
  # Synthetic pass/fail gates through the real recorder, so the self-test
  # proves the bundle fields and the failure propagation without touching the
  # real toolchain.
  gate_command() {
    case "$1" in
      selftest-pass) printf 'true' ;;
      selftest-fail) printf 'false' ;;
      *) return 1 ;;
    esac
  }
  execute_gate selftest-pass
  execute_gate selftest-fail
else
  for gate in ${CANONICAL_GATES}; do
    run_gate "${gate}"
  done
fi

# ─── Artifacts ───────────────────────────────────────────────────────────────
ARTIFACTS_JSON=""
ARTIFACTS_COUNT=0
append_artifact() { # <json-entry>
  if [ "${ARTIFACTS_COUNT}" -gt 0 ]; then
    ARTIFACTS_JSON="${ARTIFACTS_JSON},"
  fi
  ARTIFACTS_JSON="${ARTIFACTS_JSON}${1}"
  ARTIFACTS_COUNT=$((ARTIFACTS_COUNT + 1))
}
record_artifact() { # <label> <path>
  local label="$1" path="$2" digest
  if [ -f "${path}" ]; then
    digest="$(digest_file "${path}")"
    append_artifact "$(printf '{"name":"%s","path":"%s","present":true,"sha256":"%s"}' \
      "$(json_escape "${label}")" "$(json_escape "${path}")" "$(json_escape "${digest}")")"
  else
    append_artifact "$(printf '{"name":"%s","path":"%s","present":false}' \
      "$(json_escape "${label}")" "$(json_escape "${path}")")"
  fi
}
record_artifact apex-api target/release/apex-api
record_artifact apex-worker target/release/apex-worker
record_artifact tailwind-committed crates/api/static/css/tailwind.css
record_artifact tailwind-rebuilt "${out_dir}/tailwind.css"

generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if [ "${FAILED}" -ne 0 ]; then
  status="failed"
else
  status="passed"
fi
complete=false
if [ "${NOT_RUN}" -eq 0 ] && [ "${FAILED}" -eq 0 ]; then
  complete=true
fi

printf '{\n  "schema_version": %d,\n  "generated_at": "%s",\n  "git_sha": "%s",\n  "exact_sha": %s,\n  "exact_sha_head": "%s",\n  "worktree_clean": %s,\n  "pipeline_id": "%s",\n  "build_timestamp": "%s",\n  "artifact_digest": "%s",\n  "status": "%s",\n  "complete": %s,\n  "gates": [%s],\n  "artifacts": [%s]\n}\n' \
  "${SCHEMA_VERSION}" \
  "${generated_at}" \
  "$(json_escape "${sha}")" \
  "${exact_sha}" \
  "$(json_escape "${head_sha}")" \
  "${worktree_clean}" \
  "$(json_escape "${pipeline_id}")" \
  "$(json_escape "${build_timestamp}")" \
  "$(json_escape "${artifact_digest}")" \
  "${status}" \
  "${complete}" \
  "${GATES_JSON}" \
  "${ARTIFACTS_JSON}" \
  >"${out_dir}/release-evidence.json"

{
  echo "release evidence — schema ${SCHEMA_VERSION}"
  echo "generated_at:    ${generated_at}"
  echo "git_sha:         ${sha}"
  echo "exact_sha:       ${exact_sha} (HEAD ${head_sha}, worktree_clean=${worktree_clean})"
  echo "pipeline_id:     ${pipeline_id}"
  echo "build_timestamp: ${build_timestamp}"
  echo "artifact_digest: ${artifact_digest}"
  echo "status:          ${status} (complete=${complete})"
  echo
  echo "gates:"
  printf '%s' "${GATES_JSON}" | tr '}' '}\n' | grep -o '"name":"[^"]*","status":"[^"]*","source":"[^"]*"' \
    | sed -e 's/"name":"/  /' -e 's/","status":"/ -> /' -e 's/","source":"/ [/;s/$/]/'
  if [ "${FAILED}" -ne 0 ]; then
    echo
    echo "failed gates:"
    printf '%s' "${GATES_JSON}" | tr '}' '}\n' | grep -o '"name":"[^"]*","status":"failed"' \
      | sed -e 's/"name":"/  /' -e 's/","status":"failed"/ FAILED/'
  fi
} >"${out_dir}/release-evidence.txt"

# Digest the bundle files themselves, so the JSON can be matched to its
# sidecar without trusting the JSON's self-description. The sidecar uses the
# standard "<hex>  <name>" checksum format, consumable by `sha256sum -c` /
# `shasum -a 256 -c`.
{
  printf '%s  release-evidence.json\n' \
    "$(digest_file "${out_dir}/release-evidence.json" | sed 's/^sha256://')"
  printf '%s  release-evidence.txt\n' \
    "$(digest_file "${out_dir}/release-evidence.txt" | sed 's/^sha256://')"
} >"${out_dir}/release-evidence.sha256"

echo
echo "release evidence written to ${out_dir}"
echo "  release-evidence.json"
echo "  release-evidence.txt"
echo "  release-evidence.sha256"
echo
echo "Export these in the deployed service environment so /api/version matches"
echo "this bundle:"
echo "  APEX_GIT_SHA=${sha}"
echo "  APEX_BUILD_TIMESTAMP=${build_timestamp}"
echo "  APEX_CI_PIPELINE_ID=${pipeline_id}"
echo "  APEX_ARTIFACT_DIGEST=${artifact_digest}"

if [ "${self_test}" -eq 1 ]; then
  bundle="${out_dir}/release-evidence.json"
  for field in '"schema_version"' '"generated_at"' '"git_sha"' '"pipeline_id"' '"build_timestamp"' '"artifact_digest"' '"exact_sha"' '"status"' '"gates"' '"artifacts"'; do
    if ! grep -q "${field}" "${bundle}"; then
      echo "release evidence SELF-TEST FAILED: missing field ${field}" >&2
      exit 2
    fi
  done
  for expectation in \
    '"name":"selftest-pass","status":"passed","source":"executed"' \
    '"name":"selftest-fail","status":"failed","source":"executed"' \
    '"status":"failed"'; do
    if ! grep -q "${expectation}" "${bundle}"; then
      echo "release evidence SELF-TEST FAILED: expected ${expectation}" >&2
      exit 2
    fi
  done
  echo
  echo "release evidence self-test passed: fields present, failing gate recorded, exit status propagates"
fi

if [ "${FAILED}" -ne 0 ]; then
  echo "release evidence FAILED: ${EXECUTED} executed, ${RECORDED} recorded, ${NOT_RUN} not run" >&2
  exit 1
fi

echo "release evidence passed: ${EXECUTED} executed, ${RECORDED} recorded, ${NOT_RUN} not run"
