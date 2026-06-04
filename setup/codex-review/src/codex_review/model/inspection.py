"""Validation helpers for model-side repository inspection evidence."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.errors import ValidationError
from codex_review.security.redaction import assert_no_secret_patterns

EVIDENCE_KEYS = ("path", "purpose", "observation")


def validate_inspection_evidence(
    payload: dict[str, Any],
    repo_path: str | Path | None,
    context: str,
) -> list[dict[str, str]]:
    """Require non-empty, repo-local inspection evidence from model outputs."""
    raw_items = payload.get("inspection_evidence")
    if not isinstance(raw_items, list) or not raw_items:
        raise ValidationError(f"{context} requires non-empty inspection_evidence")

    root = Path(repo_path).resolve() if repo_path is not None else None
    items: list[dict[str, str]] = []
    for index, raw_item in enumerate(raw_items):
        if not isinstance(raw_item, dict):
            raise ValidationError(f"{context} inspection_evidence[{index}] must be an object")
        item: dict[str, str] = {}
        for key in EVIDENCE_KEYS:
            value = str(raw_item.get(key) or "").strip()
            if not value:
                raise ValidationError(f"{context} inspection_evidence[{index}].{key} must be non-empty")
            assert_no_secret_patterns(value, f"{context}.inspection_evidence[{index}].{key}")
            item[key] = value

        if root is not None:
            raw_evidence_path = Path(item["path"])
            candidate = raw_evidence_path.resolve() if raw_evidence_path.is_absolute() else (root / raw_evidence_path).resolve()
            try:
                evidence_path = candidate.relative_to(root)
            except ValueError as exc:
                raise ValidationError(f"{context} inspection_evidence path escapes repo: {item['path']}") from exc
            if not candidate.is_file():
                raise ValidationError(f"{context} inspection_evidence path does not exist: {item['path']}")
            item["path"] = evidence_path.as_posix()
        else:
            evidence_path = Path(item["path"])
            if evidence_path.is_absolute() or ".." in evidence_path.parts:
                raise ValidationError(f"{context} inspection_evidence path must be repo-relative: {item['path']}")
        items.append(item)
    return items
