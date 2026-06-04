"""JSON schema and lightweight validation helpers."""
from __future__ import annotations

import json
from copy import deepcopy
from typing import Any, Iterable

from codex_review.core.errors import ValidationError
from codex_review.core.paths import schema_path


def load_schema_text(name: str) -> str:
    return schema_path(name).read_text(encoding="utf-8")


def load_schema_json(name: str) -> dict[str, Any]:
    return json.loads(load_schema_text(name))


def require_schema_version(payload: dict[str, Any], expected: str) -> None:
    actual = payload.get("schema_version")
    if actual != expected:
        raise ValidationError(f"expected schema_version {expected!r}, got {actual!r}")


def validate_required_keys(payload: dict[str, Any], keys: Iterable[str], context: str = "payload") -> None:
    missing = [key for key in keys if key not in payload]
    if missing:
        raise ValidationError(f"{context} missing required keys: {', '.join(missing)}")


def validate_enum(value: Any, allowed: Iterable[Any], context: str = "value") -> None:
    allowed_set = set(allowed)
    if value not in allowed_set:
        raise ValidationError(f"{context} must be one of {sorted(map(str, allowed_set))}, got {value!r}")


def validate_json_schema(payload: dict[str, Any], schema_name: str) -> None:
    try:
        import jsonschema  # type: ignore
    except Exception:
        return
    jsonschema.validate(payload, load_schema_json(schema_name))


def _string_array_schema() -> dict[str, Any]:
    return {"type": "array", "items": {"type": "string"}}


def _loose_finding_item_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "finding_id": {"type": "string"},
            "severity": {"enum": ["critical", "high", "medium", "low", "info"]},
            "file": {"type": "string"},
            "line": {"type": "integer"},
            "root_cause_key": {"type": "string"},
            "title": {"type": "string"},
            "summary": {"type": "string"},
            "evidence": {"type": "string"},
            "axis": {"type": "string"},
            "type": {"type": "string"},
            "confidence": {"type": "number"},
        },
    }


def _design_item_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "finding_id": {"type": "string"},
            "root_cause_key": {"type": "string"},
            "invariant": {"type": "string"},
            "summary": {"type": "string"},
            "file": {"type": "string"},
            "line": {"type": "integer"},
            "evidence": {"type": "string"},
        },
    }


def _cluster_item_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "cluster_id": {"type": "string"},
            "finding_ids": _string_array_schema(),
            "root_cause_key": {"type": "string"},
            "invariant": {"type": "string"},
            "summary": {"type": "string"},
            "severity_score": {"type": "integer"},
        },
    }


def _cluster_analysis_item_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "cluster_id": {"type": "string"},
            "root_cause": {"type": "string"},
            "affected_surface": {"type": "string"},
            "retired_approaches": _string_array_schema(),
            "conflict_risks": _string_array_schema(),
            "test_needs": _string_array_schema(),
            "fix_strategy": {"type": "string"},
            "summary": {"type": "string"},
        },
    }


def _task_item_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "task_id": {"type": "string"},
            "id": {"type": "string"},
            "summary": {"type": "string"},
            "files": _string_array_schema(),
            "allowed_files": _string_array_schema(),
            "acceptance_criteria": _string_array_schema(),
            "tests": _string_array_schema(),
        },
    }


def _inspection_evidence_item_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "purpose": {"type": "string"},
            "observation": {"type": "string"},
        },
        "required": ["path", "purpose", "observation"],
        "additionalProperties": False,
    }


def _fix_policy_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "allowed_files": _string_array_schema(),
            "allowed_prefixes": _string_array_schema(),
            "forbidden_files": _string_array_schema(),
            "forbidden_prefixes": _string_array_schema(),
        },
    }


def _issue_request_schema() -> dict[str, Any]:
    return {
        "type": "object",
        "properties": {
            "title": {"type": "string"},
            "body": {"type": "string"},
            "root_cause_key": {"type": "string"},
            "labels": _string_array_schema(),
        },
    }


def _default_array_item_schema(path: tuple[str, ...]) -> dict[str, Any]:
    name = path[-1] if path else ""
    if name in {"findings", "duplicates", "inline_comments", "summary_items", "deferred_items"}:
        return _loose_finding_item_schema()
    if name == "items":
        return _design_item_schema()
    if name == "clusters":
        return _cluster_item_schema()
    if name == "analyses":
        return _cluster_analysis_item_schema()
    if name in {"edit_sequence", "task_hints", "tasks"}:
        return _task_item_schema()
    if name == "inspection_evidence":
        return _inspection_evidence_item_schema()
    if name in {
        "tests",
        "risk_flags",
        "files",
        "allowed_files",
        "allowed_prefixes",
        "forbidden_files",
        "forbidden_prefixes",
        "acceptance_criteria",
        "finding_ids",
    }:
        return {"type": "string"}
    return {"type": "string"}


def _object_override(path: tuple[str, ...]) -> dict[str, Any] | None:
    name = path[-1] if path else ""
    if name == "issue_request":
        return _issue_request_schema()
    if name == "fix_policy":
        return _fix_policy_schema()
    return None


def _add_null_type(schema: dict[str, Any]) -> dict[str, Any]:
    out = deepcopy(schema)
    if "enum" in out:
        values = list(out["enum"])
        if None not in values:
            values.append(None)
        out["enum"] = values
        _ensure_enum_type(out)
        return out
    value_type = out.get("type")
    if isinstance(value_type, list):
        if "null" not in value_type:
            out["type"] = [*value_type, "null"]
    elif isinstance(value_type, str):
        if value_type != "null":
            out["type"] = [value_type, "null"]
    else:
        out["type"] = ["object", "null"] if "properties" in out else ["string", "null"]
    return out


def _ensure_enum_type(schema: dict[str, Any]) -> None:
    if "enum" not in schema:
        return
    values = schema.get("enum") or []
    if "type" in schema:
        existing = schema["type"]
        types = list(existing) if isinstance(existing, list) else [existing]
        if any(value is None for value in values) and "null" not in types:
            types.append("null")
            schema["type"] = types[0] if len(types) == 1 else types
        return
    non_null = [value for value in values if value is not None]
    inferred: set[str] = set()
    for value in non_null:
        if isinstance(value, bool):
            inferred.add("boolean")
        elif isinstance(value, int) and not isinstance(value, bool):
            inferred.add("integer")
        elif isinstance(value, (int, float)) and not isinstance(value, bool):
            inferred.add("number")
        else:
            inferred.add("string")
    if not inferred:
        inferred.add("string")
    ordered = [kind for kind in ("string", "integer", "number", "boolean") if kind in inferred]
    if any(value is None for value in values):
        ordered.append("null")
    schema["type"] = ordered[0] if len(ordered) == 1 else ordered


def _is_object_schema(schema: dict[str, Any]) -> bool:
    return schema.get("type") == "object" or "properties" in schema


def make_openai_structured_output_schema(schema: dict[str, Any]) -> dict[str, Any]:
    """Convert a project JSON schema into OpenAI Structured Outputs strict form."""

    def convert(node: Any, path: tuple[str, ...], *, optional: bool = False) -> Any:
        if isinstance(node, list):
            return [convert(item, (*path, str(index))) for index, item in enumerate(node)]
        if not isinstance(node, dict):
            return node

        override = _object_override(path)
        current = deepcopy(override if override is not None else node)

        if current.get("type") == "array" and "items" not in current:
            current["items"] = _default_array_item_schema(path)

        if _is_object_schema(current):
            properties = current.setdefault("properties", {})
            original_required = set(current.get("required") or [])
            converted_properties: dict[str, Any] = {}
            for key, value in properties.items():
                converted = convert(value, (*path, key), optional=key not in original_required)
                if key not in original_required:
                    converted = _add_null_type(converted)
                converted_properties[key] = converted
            current["properties"] = converted_properties
            current["required"] = list(converted_properties.keys())
            current["additionalProperties"] = False
        else:
            for key, value in list(current.items()):
                if key in {"properties", "items"}:
                    continue
                current[key] = convert(value, (*path, key))

        if current.get("type") == "array":
            current["items"] = convert(current.get("items", {}), (*path, "items"))

        _ensure_enum_type(current)

        if optional:
            current = _add_null_type(current)
        return current

    strict = convert(schema, ())
    if isinstance(strict, dict):
        strict["$id"] = str(strict.get("$id", "")).replace("/schemas/", "/schemas/openai/") or strict.get("$id", "")
        strict["x-codex-review-openai-strict"] = True
    return strict
