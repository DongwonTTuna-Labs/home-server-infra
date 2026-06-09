"""Append-only review-memory.v1 ledger store."""
from __future__ import annotations

import json
from copy import deepcopy
from pathlib import Path
from typing import Any, Mapping, Sequence

from codex_review.core.artifacts import MAX_TEXT_ARTIFACT_BYTES, write_json
from codex_review.core.errors import ValidationError
from codex_review.memory.paths import ledger_path
from codex_review.memory.types import (
    SCHEMA_VERSION,
    validate_review_memory_entry,
    validate_review_memory_ledger,
)

MAX_LEDGER_BYTES = MAX_TEXT_ARTIFACT_BYTES

_DEFAULT_SCOPE: dict[str, Any] = {"repository": "", "pr_number": 0, "base_ref": ""}


def read_ledger(repo_path: str | Path, scope: Mapping[str, Any]) -> dict[str, Any]:
    """Read the PR-scoped ledger under ``repo_path`` as untrusted data."""
    normalized_scope = _normalize_scope(scope)
    path = Path(repo_path) / ledger_path(normalized_scope["repository"], normalized_scope["pr_number"])
    return read_ledger_file(path, normalized_scope)


def read_ledger_file(path: str | Path, scope: Mapping[str, Any] | None = None) -> dict[str, Any]:
    """Read a ledger file, failing safely to an empty ledger on corrupt input."""
    expected_scope = _normalize_scope(scope) if scope is not None else None
    fallback_scope = expected_scope or _normalize_scope(None)
    ledger_file = Path(path)

    if not ledger_file.exists():
        return _empty_ledger(fallback_scope)

    try:
        data = ledger_file.read_bytes()
    except OSError as exc:
        return _empty_ledger(fallback_scope, _warning("read_error", f"could not read ledger: {exc}", ledger_file))

    if len(data) > MAX_LEDGER_BYTES:
        return _empty_ledger(
            fallback_scope,
            _warning("oversized", f"ledger exceeds {MAX_LEDGER_BYTES} bytes", ledger_file),
        )

    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as exc:
        return _empty_ledger(fallback_scope, _warning("invalid_utf8", f"ledger is not UTF-8: {exc}", ledger_file))

    try:
        payload = json.loads(text)
    except json.JSONDecodeError as exc:
        return _empty_ledger(fallback_scope, _warning("malformed_json", f"ledger JSON is malformed: {exc}", ledger_file))

    if not isinstance(payload, dict):
        return _empty_ledger(fallback_scope, _warning("non_object", "ledger JSON must be an object", ledger_file))

    ledger = deepcopy(payload)
    try:
        validate_review_memory_ledger(ledger)
    except Exception as exc:
        return _empty_ledger(fallback_scope, _warning("invalid_ledger", f"ledger failed schema validation: {exc}", ledger_file))

    if expected_scope is not None and _normalize_scope(ledger.get("scope")) != expected_scope:
        return _empty_ledger(expected_scope, _warning("scope_mismatch", "ledger scope does not match requested scope", ledger_file))

    return ledger


def append_entries(
    ledger: Mapping[str, Any],
    entries: Sequence[Mapping[str, Any]],
    max_entries: int | None = None,
) -> dict[str, Any]:
    """Return a new ledger with valid, non-duplicate entries appended."""
    current = _validated_core_ledger(ledger)
    existing_entries = deepcopy(current["entries"])
    seen_entry_ids = {entry.get("entry_id") for entry in existing_entries}

    appended: list[dict[str, Any]] = []
    for raw_entry in entries:
        entry = deepcopy(dict(raw_entry))
        try:
            validate_review_memory_entry(entry)
        except ValidationError:
            raise
        except Exception as exc:
            if _is_jsonschema_error(exc):
                raise ValidationError(f"review memory entry failed schema validation: {exc}") from exc
            raise
        entry_id = entry["entry_id"]
        if entry_id in seen_entry_ids:
            continue
        seen_entry_ids.add(entry_id)
        appended.append(entry)

    next_entries = [*existing_entries, *appended]
    if max_entries is not None:
        cap = _normalize_max_entries(max_entries)
        next_entries = next_entries[-cap:] if cap else []

    out = {"schema_version": SCHEMA_VERSION, "scope": deepcopy(current["scope"]), "entries": next_entries}
    _validate_ledger(out, "review memory ledger")
    return out


def write_ledger(out_path: str | Path, ledger: Mapping[str, Any]) -> Path:
    """Validate and write a schema-stamped ledger JSON artifact."""
    if not isinstance(ledger, Mapping):
        raise ValidationError("review memory ledger must be an object")
    payload = deepcopy(dict(ledger))
    if "schema_version" not in payload:
        payload["schema_version"] = SCHEMA_VERSION
    _validate_ledger(payload, "review memory ledger")
    return write_json(out_path, payload, SCHEMA_VERSION)


def _validated_core_ledger(ledger: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(ledger, Mapping):
        raise ValidationError("review memory ledger must be an object")
    payload = {
        "schema_version": ledger.get("schema_version", SCHEMA_VERSION),
        "scope": deepcopy(ledger.get("scope")),
        "entries": deepcopy(ledger.get("entries", [])),
    }
    _validate_ledger(payload, "review memory ledger")
    return payload


def _validate_ledger(payload: Mapping[str, Any], context: str) -> None:
    try:
        validate_review_memory_ledger(payload)
    except ValidationError:
        raise
    except Exception as exc:
        if _is_jsonschema_error(exc):
            raise ValidationError(f"{context} failed schema validation: {exc}") from exc
        raise


def _is_jsonschema_error(exc: Exception) -> bool:
    return exc.__class__.__module__.startswith("jsonschema")


def _empty_ledger(scope: Mapping[str, Any] | None, warning: dict[str, Any] | None = None) -> dict[str, Any]:
    ledger = {"schema_version": SCHEMA_VERSION, "scope": _normalize_scope(scope), "entries": []}
    if warning is not None:
        ledger["warnings"] = [warning]
    return ledger


def _normalize_scope(scope: Mapping[str, Any] | None) -> dict[str, Any]:
    if not isinstance(scope, Mapping):
        return dict(_DEFAULT_SCOPE)
    raw_pr_number = scope.get("pr_number", 0)
    try:
        pr_number = int(raw_pr_number)
    except (TypeError, ValueError):
        pr_number = 0
    if isinstance(raw_pr_number, bool):
        pr_number = 0
    return {
        "repository": str(scope.get("repository") or ""),
        "pr_number": pr_number,
        "base_ref": str(scope.get("base_ref") or ""),
    }


def _warning(code: str, message: str, path: Path) -> dict[str, Any]:
    return {"code": code, "message": message, "path": str(path)}


def _normalize_max_entries(max_entries: int) -> int:
    if isinstance(max_entries, bool):
        raise ValidationError("max_entries must be an integer")
    try:
        cap = int(max_entries)
    except (TypeError, ValueError):
        raise ValidationError("max_entries must be an integer") from None
    if cap < 0:
        raise ValidationError("max_entries must be non-negative")
    return cap
