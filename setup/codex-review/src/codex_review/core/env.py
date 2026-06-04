"""Environment and event helpers."""
from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any

from codex_review.core.errors import ValidationError


def require_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise ValidationError(f"required environment variable is missing: {name}")
    return value


def read_github_env() -> dict[str, str | None]:
    keys = [
        "GITHUB_REPOSITORY",
        "GITHUB_EVENT_NAME",
        "GITHUB_EVENT_PATH",
        "GITHUB_ACTOR",
        "GITHUB_TRIGGERING_ACTOR",
        "GITHUB_SHA",
        "GITHUB_REF",
        "GITHUB_RUN_ID",
        "GITHUB_OUTPUT",
        "GITHUB_STEP_SUMMARY",
    ]
    return {key: os.environ.get(key) for key in keys}


def read_event_payload(path: str | Path | None = None) -> dict[str, Any]:
    p = Path(path or require_env("GITHUB_EVENT_PATH"))
    try:
        payload = json.loads(p.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValidationError(f"GitHub event payload not found: {p}") from None
    except json.JSONDecodeError as exc:
        raise ValidationError(f"GitHub event payload is malformed: {exc}") from exc
    if not isinstance(payload, dict):
        raise ValidationError("GitHub event payload must be an object")
    return payload


def resolve_repository_parts(value: str | None = None) -> tuple[str, str]:
    repo = value or os.environ.get("GITHUB_REPOSITORY", "")
    if "/" not in repo:
        raise ValidationError(f"repository must be owner/repo: {repo!r}")
    owner, name = repo.split("/", 1)
    if not owner or not name:
        raise ValidationError(f"repository must be owner/repo: {repo!r}")
    return owner, name
