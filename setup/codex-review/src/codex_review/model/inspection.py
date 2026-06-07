"""Validation helpers for model-side repository inspection evidence."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.errors import ValidationError
from codex_review.security.redaction import assert_no_secret_patterns

EVIDENCE_KEYS = ("path", "purpose", "observation")


def collect_existing_evidence_paths(*payloads: dict[str, Any]) -> list[str]:
    """Gather unique inspection_evidence paths already produced by upstream stages.

    Upstream artifacts (inventory, techlead decision, design plan) only pass
    validation when every evidence path is an existing repo file, so their paths
    are a safe candidate set to hand downstream models for re-citation.
    """
    seen: list[str] = []
    for payload in payloads:
        if not isinstance(payload, dict):
            continue
        for item in payload.get("inspection_evidence") or []:
            if not isinstance(item, dict):
                continue
            path = str(item.get("path") or "").strip()
            if path and path not in seen:
                seen.append(path)
    return seen


def render_evidence_citation_hint(paths: list[str]) -> str:
    """Prompt fragment steering models to cite existing files, never a target.

    The recurring failure is a model citing the file a plan proposes to create
    (which does not exist yet) as inspection_evidence; the validator then rejects
    it. This reminds the model to cite the existing spec/task that requires the
    file and shows the correct vs incorrect shape.
    """
    lines = [
        "\nNEVER cite an inspection_evidence.path that does not already exist in "
        "pr-head -- especially a file your plan proposes to CREATE. Cite the "
        "existing spec/task/design/proposal file that proves the change is "
        "required and name the missing/target file in observation instead.",
    ]
    if paths:
        joined = ", ".join(paths)
        lines.append(
            "Prefer citing these files that earlier stages already verified to "
            f"exist: {joined}."
        )
    lines.append(
        'Example -- CORRECT: {"path":"openspec/changes/foo/spec.md",'
        '"observation":"requires docs/foo.md, which is absent"}; '
        'WRONG: {"path":"docs/foo.md","observation":"file is missing"} '
        "(docs/foo.md does not exist yet)."
    )
    return "\n".join(lines)


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
