#!/usr/bin/env bash
# CI guard: every `#[ignore = "requires PostgreSQL"]` test target in the
# workspace must be registered with the canonical PostgreSQL suite runner.
#
# Why this exists
# ---------------
# The integration suites are `#[ignore]`d so `cargo test` stays hermetic, but
# that also means a new suite silently never runs in CI unless someone
# remembers to add it to `scripts/ci/run_pg_integration_suites.sh`. This script
# enumerates the ignored PostgreSQL tests from the source tree and compares them
# against the registrations in the runner (and confirms .woodpecker.yml invokes
# the runner), failing loudly on any target CI would skip.
#
# Registration rules
# ------------------
# - `crates/<dir>/tests/<name>.rs` requires a runner command containing
#   `-p <package>`, `--test <name>` and `--ignored`.
# - `crates/<dir>/src/**` requires a runner command containing `-p <package>`
#   and `--ignored` with no `--test` filter (the whole unit-test surface of the
#   package is exercised, e.g. `cargo test -p apex-insights --lib ... --ignored`).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

RUNNER="scripts/ci/run_pg_integration_suites.sh"
PIPELINE=".woodpecker.yml"

failures=0
checked=0

if [[ ! -f "${RUNNER}" ]]; then
  echo "missing canonical runner ${RUNNER}" >&2
  exit 1
fi
if ! grep -q 'run_pg_integration_suites.sh' "${PIPELINE}"; then
  echo "${PIPELINE} does not invoke ${RUNNER}" >&2
  exit 1
fi

# Join a file's backslash-continued command lines into one logical line each.
logical_lines() {
  awk '
    /\\$/ { pending = pending substr($0, 1, length($0) - 1) " "; next }
    { print pending $0; pending = "" }
    END { if (pending != "") print pending }
  ' "$1"
}

# package_name <crate_dir> -> Cargo package name from crates/<dir>/Cargo.toml.
package_name() {
  sed -n 's/^name[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "crates/$1/Cargo.toml" | head -n 1
}

registered_test_target() { # <package> <test-target>
  local pkg="$1" target="$2" line
  while IFS= read -r line; do
    case "${line}" in
      *"cargo test"*)
        case "${line}" in *"-p ${pkg}"*) ;; *) continue ;; esac
        case "${line}" in *"--test ${target}"*) ;; *) continue ;; esac
        case "${line}" in *"--ignored"*) return 0 ;; esac
        ;;
    esac
  done < <(logical_lines "${RUNNER}")
  return 1
}

registered_unit_target() { # <package>
  local pkg="$1" line
  while IFS= read -r line; do
    case "${line}" in
      *"cargo test"*)
        case "${line}" in *"-p ${pkg}"*) ;; *) continue ;; esac
        case "${line}" in *"--ignored"*) ;; *) continue ;; esac
        case "${line}" in *"--test "*) continue ;; esac
        return 0
        ;;
    esac
  done < <(logical_lines "${RUNNER}")
  return 1
}

while IFS= read -r file; do
  # Only files that actually contain an ignored PostgreSQL test attribute.
  grep -q '#\[ignore = "requires PostgreSQL' "${file}" || continue

  rel="${file#./}"
  crate_dir="$(printf '%s' "${rel}" | cut -d/ -f2)"
  pkg="$(package_name "${crate_dir}")"
  if [[ -z "${pkg}" ]]; then
    echo "cannot resolve cargo package for ${rel}" >&2
    failures=$((failures + 1))
    continue
  fi

  case "${rel}" in
    crates/*/tests/*.rs)
      target="$(basename "${rel}" .rs)"
      checked=$((checked + 1))
      if ! registered_test_target "${pkg}" "${target}"; then
        echo "UNREGISTERED PostgreSQL integration target: ${pkg} --test ${target} (${rel})" >&2
        failures=$((failures + 1))
      fi
      ;;
    crates/*/src/*.rs)
      checked=$((checked + 1))
      if ! registered_unit_target "${pkg}"; then
        echo "UNREGISTERED PostgreSQL unit-test target: ${pkg} (${rel})" >&2
        failures=$((failures + 1))
      fi
      ;;
    *)
      echo "ignored PostgreSQL test in unsupported path: ${rel}" >&2
      failures=$((failures + 1))
      ;;
  esac
done < <(find crates -path '*/target/*' -prune -o -name '*.rs' -print | sort)

if [[ "${checked}" -eq 0 ]]; then
  echo "no ignored PostgreSQL test targets found — did the ignore attribute change?" >&2
  exit 1
fi

if [[ "${failures}" -gt 0 ]]; then
  echo >&2
  echo "${failures} ignored PostgreSQL test target(s) are not registered in ${RUNNER}." >&2
  echo "Add them to the canonical runner (and keep .woodpecker.yml invoking it)." >&2
  exit 1
fi

echo "PostgreSQL suite registration OK: ${checked} ignored test target(s) all run in CI."
