"""Artifact IO helpers."""
from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any, Iterable

from codex_review.core.errors import ValidationError
from codex_review.core.schema import require_schema_version

MAX_TEXT_ARTIFACT_BYTES = 5_000_000


def _path(path: str | Path) -> Path:
    return Path(path)


def read_json(path: str | Path, expected_schema_version: str | None = None) -> dict[str, Any]:
    p = _path(path)
    try:
        payload = json.loads(p.read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValidationError(f"missing JSON artifact: {p}") from None
    except json.JSONDecodeError as exc:
        raise ValidationError(f"malformed JSON artifact {p}: {exc}") from exc
    if not isinstance(payload, dict):
        raise ValidationError(f"JSON artifact must be an object: {p}")
    if expected_schema_version is not None:
        require_schema_version(payload, expected_schema_version)
    return payload


def write_json(path: str | Path, payload: dict[str, Any], schema_version: str | None = None) -> Path:
    if not isinstance(payload, dict):
        raise ValidationError("write_json payload must be a dict")
    out = dict(payload)
    if schema_version is not None:
        existing = out.get("schema_version")
        if existing is not None and existing != schema_version:
            raise ValidationError(f"schema_version mismatch: {existing} != {schema_version}")
        out["schema_version"] = schema_version
    p = _path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(out, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return p


def read_text(path: str | Path, max_bytes: int = MAX_TEXT_ARTIFACT_BYTES) -> str:
    p = _path(path)
    data = p.read_bytes()
    if len(data) > max_bytes:
        raise ValidationError(f"text artifact too large: {p} bytes={len(data)} limit={max_bytes}")
    return data.decode("utf-8")


def write_text(path: str | Path, content: str) -> Path:
    p = _path(path)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(content, encoding="utf-8")
    return p


def require_artifacts(paths: Iterable[str | Path]) -> list[Path]:
    missing = [str(p) for p in map(Path, paths) if not p.exists()]
    if missing:
        raise ValidationError(f"missing required artifacts: {', '.join(missing)}")
    return [Path(p) for p in paths]


def hash_artifact(path: str | Path) -> str:
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()
