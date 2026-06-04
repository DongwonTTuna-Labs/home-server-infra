"""Review axis helpers."""
from __future__ import annotations
from codex_review.core.errors import ValidationError

DEFAULT_AXES=["correctness","security","performance","test-coverage","domain"]

def review_axes(config: dict | None = None) -> list[str]:
    return list((config or {}).get("review", {}).get("axes") or DEFAULT_AXES)

def validate_axis(axis: str, config: dict | None = None) -> None:
    if axis not in review_axes(config):
        raise ValidationError(f"unknown review axis: {axis}")

def axis_artifact_name(axis: str) -> str:
    return f"review-{axis}-findings.json"
