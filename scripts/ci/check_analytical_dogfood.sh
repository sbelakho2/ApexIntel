#!/usr/bin/env bash
# Adversarial dogfood for the analytical-excellence pipeline.
#
# Runs the deterministic attack suite (fabricated statistics, number
# manipulation, temporal fabrication, citation injection, poisoned
# corroboration, single-source overclaim, ignored contradictions, shallow
# filler, recommendation smuggling, org hallucination, red-team attacks) and
# 400 mutation-fuzz iterations. Any attack that escapes fails the build.
#
# The harness is pure (no network, no database) and finishes in ~1s after a
# build, so it is cheap enough for every CI run.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

cargo run --locked -q -p apex-worker --example analytical_adversarial_dogfood
