"""Semantic commit plan validation for autofix pushes."""
from __future__ import annotations

import re
from typing import Any

from codex_review.core.errors import ValidationError


CONVENTIONAL_SUBJECT_RE = re.compile(r"^[a-z]+(?:\([^)]+\))?: .{8,}$")
GENERIC_SUBJECTS = {"codex review autofix", "autofix", "automated fix"}


def extract_patch_paths(patch_text: str) -> list[str]:
    paths: list[str] = []
    seen: set[str] = set()
    for line in (patch_text or "").splitlines():
        if not line.startswith("diff --git "):
            continue
        parts = line.split()
        if len(parts) < 4:
            continue
        for raw in (parts[2], parts[3]):
            if raw.startswith("b/"):
                path = raw[2:]
                if path and path not in seen and path != "/dev/null":
                    seen.add(path)
                    paths.append(path)
                break
    return paths


def validate_commit_subject(subject: Any) -> str:
    text = str(subject or "").strip()
    if not text:
        raise ValidationError("commit plan subject is required")
    if text.lower() in GENERIC_SUBJECTS:
        raise ValidationError("commit plan subject must describe the actual fix, not generic autofix")
    if text == "Codex Review Autofix" or "codex review autofix" in text.lower():
        raise ValidationError("commit plan subject must not use Codex Review Autofix")
    if len(text) > 100:
        raise ValidationError("commit plan subject must be 100 characters or fewer")
    if not CONVENTIONAL_SUBJECT_RE.match(text):
        raise ValidationError("commit plan subject must use Conventional Commit format")
    return text


def validate_commit_paths(paths: Any) -> list[str]:
    if not isinstance(paths, list) or not paths:
        raise ValidationError("commit plan entry requires non-empty paths")
    out: list[str] = []
    seen: set[str] = set()
    for value in paths:
        path = str(value or "").strip()
        if not path:
            raise ValidationError("commit plan path must not be empty")
        if path.startswith("/") or ".." in path.split("/"):
            raise ValidationError(f"commit plan path is outside repository scope: {path}")
        if path not in seen:
            seen.add(path)
            out.append(path)
    return out


def normalize_commit_plan(raw_plan: Any, patch_text: str | None = None) -> list[dict[str, Any]]:
    if not isinstance(raw_plan, list) or not raw_plan:
        raise ValidationError("approved autofix requires a non-empty semantic commit_plan")
    normalized: list[dict[str, Any]] = []
    covered: set[str] = set()
    for raw in raw_plan:
        if not isinstance(raw, dict):
            raise ValidationError("commit plan entries must be objects")
        subject = validate_commit_subject(raw.get("subject"))
        paths = validate_commit_paths(raw.get("paths"))
        covered.update(paths)
        normalized.append(
            {
                "subject": subject,
                "body": str(raw.get("body") or "").strip(),
                "paths": paths,
            }
        )
    if patch_text is not None:
        patch_paths = set(extract_patch_paths(patch_text))
        missing = sorted(patch_paths - covered)
        extra = sorted(covered - patch_paths)
        if missing:
            raise ValidationError(f"commit plan does not cover patch paths: {', '.join(missing)}")
        if extra:
            raise ValidationError(f"commit plan references paths not changed by patch: {', '.join(extra)}")
    return normalized
