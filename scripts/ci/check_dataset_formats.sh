#!/usr/bin/env bash
# Dataset format guard: `training_data/tenders/cpv_codes_2008.csv` must be a
# real CSV export of the official CPV 2008 list (9,454 eight-digit codes), not
# a saved HTML page or a truncated/partial download. The previous revision of
# this file was a saved web page, which silently poisoned every consumer.
#
# Contract enforced here:
#   * first line is exactly `code,description` (a trailing CR is tolerated so
#     both LF and CRLF checkouts pass)
#   * exactly 9,455 total lines: the header plus 9,454 data rows
#   * every data line starts with exactly 8 digits followed by a comma
#   * the file contains no `<!DOCTYPE` / `<html`
#   * every description is non-empty (quoted descriptions are allowed because
#     official descriptions contain commas)
#
# Dependency-light on purpose: awk/grep only, no python, so the alpine CI
# steps can run it directly.
#
# Run locally with: bash scripts/ci/check_dataset_formats.sh
# npm alias:        npm run check:datasets
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

CSV="training_data/tenders/cpv_codes_2008.csv"
EXPECTED_HEADER="code,description"
EXPECTED_ROWS=9454

if [[ ! -f "$CSV" ]]; then
  echo "dataset check: missing ${CSV}" >&2
  exit 1
fi

# The saved-web-page failure this guard exists for; catch it explicitly so the
# diagnostic names the real problem instead of a pile of malformed rows.
if grep -qE '<!DOCTYPE|<html' "$CSV"; then
  echo "dataset check: ${CSV} contains HTML markup (saved web page, not a CSV)" >&2
  exit 1
fi

# Single pass: validate the header, every data row and the exact line count,
# reporting the first few violations instead of only a count.
awk -v header="$EXPECTED_HEADER" -v expected_rows="$EXPECTED_ROWS" '
function problem(msg, line) {
  problems++
  if (problems <= 10) {
    if (line == "") printf "  %s\n", msg > "/dev/stderr"
    else printf "  line %d: %s\n", line, msg > "/dev/stderr"
  }
}
{
  sub(/\r$/, "")
}
NR == 1 {
  if ($0 != header) problem("header must be exactly \"" header "\" (got \"" $0 "\")", NR)
  next
}
{
  if ($0 !~ /^[0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9],/) {
    problem("data line must start with 8 digits followed by a comma", NR)
    next
  }
  description = substr($0, 10)
  if (substr(description, 1, 1) == "\"") {
    if (length(description) < 2 || substr(description, length(description), 1) != "\"") {
      problem("unterminated quoted description", NR)
      next
    }
    description = substr(description, 2, length(description) - 2)
  }
  if (description ~ /^[[:space:]]*$/) problem("description is empty", NR)
}
END {
  if (NR != expected_rows + 1) {
    printf "  expected %d lines (header + %d rows), got %d\n", expected_rows + 1, expected_rows, NR > "/dev/stderr"
    problems++
  }
  if (problems > 0) {
    printf "dataset check: %s failed with %d violation(s)\n", FILENAME, problems > "/dev/stderr"
    exit 1
  }
  printf "dataset check: %s OK (%d rows, 8-digit codes, non-empty descriptions)\n", FILENAME, NR - 1
}
' "$CSV"
