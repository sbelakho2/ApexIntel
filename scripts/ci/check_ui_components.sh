#!/usr/bin/env bash
# Global UI-component integrity scan.
#
# Shared controls must have exactly ONE definition. Historically the insight
# bookmark control was copy-pasted into three templates plus the handler, and
# the copies drifted (different targets, extra wrappers, aria-label overrides).
# This scan fails when a control's markup appears outside its component file,
# so a "local fix" in a page cannot silently fork the component again.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
violations=0

check_single_definition() {
  local label="$1" pattern="$2" component="$3"
  local hits
  hits="$(grep -rnE "$pattern" \
      "$root/crates/api/templates" "$root/crates/api/src" \
      --include='*.html' --include='*.rs' 2>/dev/null \
    | grep -v "$component" || true)"
  if [[ -n "$hits" ]]; then
    echo "ERROR: $label is defined outside $component:" >&2
    echo "$hits" | sed 's/^/  /' >&2
    violations=$((violations + 1))
  fi
}

check_single_definition \
  "the insight bookmark control" \
  'hx-post="/insights/[^"]*/bookmark' \
  'crates/api/templates/components/bookmark_button.html'

if [[ "$violations" -gt 0 ]]; then
  echo "ui-component scan failed: $violations duplicated control definition(s)" >&2
  exit 1
fi
echo "ui-component scan: all shared controls have a single definition"
