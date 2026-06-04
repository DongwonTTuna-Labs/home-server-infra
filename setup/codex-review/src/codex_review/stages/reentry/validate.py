"""Validate reentry loop reentry records."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError


VALID_NEXT_ENTRIES = {"resolve_gate_on_synchronize", "none", "resolve_gate"}


def validate_reentry_record(record: dict[str, Any], previous_loop_state: dict[str, Any] | None = None) -> dict[str, Any]:
    if record.get("schema_version") and record.get("schema_version") != "reentry-loop-state.v1":
        raise ValidationError("invalid reentry reentry schema_version")
    out = dict(record)
    out["schema_version"] = "reentry-loop-state.v1"
    if "pushed" not in out:
        raise ValidationError("reentry reentry record missing pushed")
    pushed = bool(out.get("pushed"))
    next_entry = out.get("next_entry") or ("resolve_gate_on_synchronize" if pushed else "none")
    if next_entry not in VALID_NEXT_ENTRIES:
        raise ValidationError(f"invalid reentry next_entry: {next_entry}")
    if pushed and not out.get("commit_sha"):
        raise ValidationError("pushed reentry record requires commit_sha")
    if pushed and not out.get("persisted") and not out.get("persistence_optional"):
        raise ValidationError("pushed reentry record must be persisted for the next workflow run")
    if not pushed and next_entry not in {"none"}:
        raise ValidationError("non-pushed reentry must not expect a synchronize run")
    loop_state = out.get("loop_state") or previous_loop_state or {}
    if loop_state and not isinstance(loop_state, dict):
        raise ValidationError("loop_state must be an object")
    out["next_entry"] = next_entry
    out["loop_state"] = loop_state
    return out


def write_validated_reentry_record(record: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, record, "reentry-loop-state.v1")
