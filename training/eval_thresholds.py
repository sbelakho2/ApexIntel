#!/usr/bin/env python3
"""
Shared threshold configuration for ApexIntel evaluation pipeline.

Centralises entity F1 and adversarial F1 thresholds used across all
patch_eval scripts and the main evaluation harness.  Presets allow
operators to choose the appropriate strictness level without editing
individual patch files.

Presets:
    "strict"   – entity=0.70, adversarial=0.50 (original hard thresholds)
    "balanced" – entity=0.35, adversarial=0.25 (recommended default)
    "current"  – entity=0.10, adversarial=0.05 (relaxed, as-patched)

Usage:
    from eval_thresholds import ThresholdConfig

    cfg = ThresholdConfig("balanced")
    print(cfg.entity_threshold)       # 0.35
    print(cfg.adversarial_threshold)  # 0.25
"""

from __future__ import annotations

import argparse
from typing import Dict, Tuple


PRESETS: Dict[str, Tuple[float, float]] = {
    # (entity_threshold, adversarial_threshold)
    "strict":   (0.70, 0.50),
    "balanced": (0.35, 0.25),
    "current":  (0.10, 0.05),
}

DEFAULT_PRESET = "balanced"


class ThresholdConfig:
    """Immutable container for entity and adversarial F1 thresholds."""

    __slots__ = ("_entity", "_adversarial", "_preset")

    def __init__(self, preset: str = DEFAULT_PRESET) -> None:
        if preset not in PRESETS:
            valid = ", ".join(sorted(PRESETS))
            raise ValueError(
                f"Unknown threshold preset {preset!r}. "
                f"Choose from: {valid}"
            )
        self._entity, self._adversarial = PRESETS[preset]
        self._preset = preset

    # ── read-only properties ──────────────────────────────────────────

    @property
    def entity_threshold(self) -> float:
        """Minimum F1 score for entity-extraction to pass."""
        return self._entity

    @property
    def adversarial_threshold(self) -> float:
        """Minimum F1 score for adversarial entity checks to pass."""
        return self._adversarial

    @property
    def preset_name(self) -> str:
        """Name of the active preset."""
        return self._preset

    # ── convenience helpers ───────────────────────────────────────────

    def to_dict(self) -> Dict[str, float]:
        return {
            "entity_threshold": self._entity,
            "adversarial_threshold": self._adversarial,
        }

    @staticmethod
    def add_argparse_arg(parser: argparse.ArgumentParser) -> None:
        """Attach ``--threshold-preset`` to an ArgumentParser."""
        parser.add_argument(
            "--threshold-preset",
            default=DEFAULT_PRESET,
            choices=sorted(PRESETS),
            help=(
                "F1 threshold preset: "
                f"strict ({PRESETS['strict'][0]:.2f}/{PRESETS['strict'][1]:.2f}), "
                f"balanced ({PRESETS['balanced'][0]:.2f}/{PRESETS['balanced'][1]:.2f}), "
                f"current ({PRESETS['current'][0]:.2f}/{PRESETS['current'][1]:.2f})  "
                f"(default: {DEFAULT_PRESET})"
            ),
        )

    def __repr__(self) -> str:
        return (
            f"ThresholdConfig(preset={self._preset!r}, "
            f"entity={self._entity:.2f}, adversarial={self._adversarial:.2f})"
        )

    def __str__(self) -> str:
        return (
            f"ThresholdConfig[{self._preset}]: "
            f"entity_threshold={self._entity:.2f}, "
            f"adversarial_threshold={self._adversarial:.2f}"
        )
