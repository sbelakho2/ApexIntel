#!/usr/bin/env bash
# Alert-publish source guard.
#
# Every alert that reaches a user must be committed to `event_outbox` in the
# same transaction as its source row and published by the canonical outbox
# drain. A direct `NatsPublisher::publish_alert(...)` call from a job or
# pipeline bypasses the transactional outbox: a crash can lose the alert, there
# is no retry/dead-letter state, no stable `Nats-Msg-Id`, and rules/triage can
# be skipped.
#
# Rule: `publish_alert(...)` (and its transport-internal variant
# `publish_alert_with_msg_id(...)`) may appear ONLY in:
#   - crates/worker/src/nats_stream.rs     (the NatsPublisher definition+tests)
#   - crates/worker/src/alert_transport.rs (the single transport seam)
#
# In particular, no file under `crates/worker/src/job_execution/` may call it;
# jobs enqueue through the outbox (or `notification_delivery`) instead.
#
# `ALERT_PUBLISH_GUARD_ROOT` overrides the scanned root; the worker integration
# test `crates/worker/tests/alert_publish_guard.rs` uses it to prove the guard
# fails on a violating fixture tree.
#
# Run from anywhere; operates on the working tree.
set -euo pipefail

ROOT="${ALERT_PUBLISH_GUARD_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
cd "${ROOT}"

SCAN_ROOT="crates"
TRANSPORT="crates/worker/src/alert_transport.rs"
PUBLISHER="crates/worker/src/nats_stream.rs"
JOBS_DIR="crates/worker/src/job_execution"
# The guard's own regression test embeds violating fixture source as strings;
# it is not production code and is excluded from the scan.
GUARD_TEST="crates/worker/tests/alert_publish_guard.rs"

PATTERN='publish_alert(_with_msg_id)?\('

failed=0

# ── 1. job_execution must be completely clean ─────────────────────────────
while IFS= read -r match; do
  [ -z "${match}" ] && continue
  echo "ALERT PUBLISH: job calls publish_alert(...) directly: ${match}" >&2
  failed=1
done < <(grep -RInE --include='*.rs' "${PATTERN}" "${JOBS_DIR}" 2>/dev/null || true)

# ── 2. The rest of the source: only the transport/publisher modules ───────
while IFS= read -r match; do
  [ -z "${match}" ] && continue
  echo "ALERT PUBLISH: publish_alert(...) outside ${TRANSPORT}/${PUBLISHER}: ${match}" >&2
  failed=1
done < <(grep -RInE --include='*.rs' "${PATTERN}" "${SCAN_ROOT}" 2>/dev/null \
           | grep -v "^${TRANSPORT}:" | grep -v "^${PUBLISHER}:" \
           | grep -v "^${GUARD_TEST}:" || true)

if [ "${failed}" -ne 0 ]; then
  echo "alert-publish check FAILED — enqueue alerts through the event outbox instead" >&2
  exit 1
fi
echo "alert-publish check passed (publish_alert confined to ${TRANSPORT})"
