#!/usr/bin/env bash
# Container-level browser integration check.
#
# Builds the worker image's `browser-check` stage (the real runtime image plus
# Chromium) and renders a JS-only fixture with late network activity and
# lazy-loaded DOM through the persistent Chromium renderer the worker uses.
# Fails when the rendered DOM is missing either marker or the readiness
# contract was not honoured.
#
# Requirements: a working docker daemon. Chromium keeps its sandbox (the image
# never passes --no-sandbox), so the sandbox needs the namespace privileges a
# container does not get by default: the run below grants SYS_ADMIN and lifts
# seccomp. That only applies to this throwaway test container.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

if ! command -v docker >/dev/null 2>&1; then
  echo "docker is required to run the worker browser container check" >&2
  exit 1
fi

IMAGE="${APEX_BROWSER_CHECK_IMAGE:-apexintel-worker-browser-check}"

docker build --target browser-check -f Dockerfile.worker -t "${IMAGE}" .
docker run --rm \
  --cap-add=SYS_ADMIN \
  --security-opt seccomp=unconfined \
  "${IMAGE}"
