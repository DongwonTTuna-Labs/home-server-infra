"""Resolved finding unification helpers for review memory."""
from __future__ import annotations

from copy import deepcopy
from typing import Any, Mapping, Sequence

from codex_review.memory.provenance import is_trusted_for_suppression
from codex_review.memory.types import SCHEMA_VERSION, is_entry_valid, validate_review_memory_entry


class _TrustedResolveGateResolvedEntry(dict[str, Any]):
    """In-memory marker for entries produced by the trusted resolve-gate path."""


def exact_finding_fingerprint(payload: Mapping[str, Any]) -> str:
    """Return the exact suppression identity for current and legacy findings."""
    return str(payload.get("finding_fingerprint") or payload.get("root_cause_key") or "").strip()


def resolve_gate_resolved_memory_as_ledger(
    resolved_memory: Mapping[str, Any] | Sequence[Mapping[str, Any]] | None,
    *,
    suppress_states: set[str] | frozenset[str] | Sequence[str],
) -> dict[str, Any]:
    """Bridge trusted resolve-gate resolved-memory artifacts into a ledger view."""
    artifact = _artifact_mapping(resolved_memory)
    return {
        "schema_version": SCHEMA_VERSION,
        "scope": _scope_from_artifact(artifact),
        "entries": resolve_gate_resolved_memory_entries(resolved_memory, suppress_states=suppress_states),
    }


def resolve_gate_resolved_memory_entries(
    resolved_memory: Mapping[str, Any] | Sequence[Mapping[str, Any]] | None,
    *,
    suppress_states: set[str] | frozenset[str] | Sequence[str],
) -> list[dict[str, Any]]:
    """Convert suppressible trusted resolve-gate items to resolved_finding entries."""
    artifact = _artifact_mapping(resolved_memory)
    allowed_states = {str(state) for state in suppress_states}
    source_schema_version = str(artifact.get("schema_version") or "")
    entries: list[dict[str, Any]] = []

    for index, item in enumerate(_items_from_artifact(resolved_memory)):
        state = str(item.get("state") or "").strip()
        if state not in allowed_states:
            continue
        fingerprint = exact_finding_fingerprint(item)
        if not fingerprint:
            continue
        entry = _TrustedResolveGateResolvedEntry(
            {
                "entry_id": _entry_id(item, fingerprint, index),
                "created_at": _created_at(item),
                "round": _round(item),
                "head_sha": str(item.get("head_sha") or artifact.get("head_sha") or ""),
                "kind": "resolved_finding",
                "category": "learnings",
                "body": _entry_body(item, state, source_schema_version),
                "source_stage": "resolve_gate",
                "trusted": True,
                "finding_fingerprint": fingerprint,
                "provenance": _resolve_gate_provenance(source_schema_version),
            }
        )
        validate_review_memory_entry(entry)
        entries.append(entry)
    return entries


def resolved_findings_for_suppression(ledger: Mapping[str, Any] | Sequence[Mapping[str, Any]] | None) -> list[dict[str, Any]]:
    """Return trusted resolved_finding entries eligible for exact-fingerprint use."""
    out: list[dict[str, Any]] = []
    for raw_entry in _entries_from_ledger(ledger):
        if not isinstance(raw_entry, Mapping):
            continue
        if raw_entry.get("kind") != "resolved_finding":
            continue
        fingerprint = exact_finding_fingerprint(raw_entry)
        if not fingerprint:
            continue
        entry = deepcopy(dict(raw_entry))
        if not is_entry_valid(entry):
            continue
        if is_trusted_for_suppression(entry) or isinstance(raw_entry, _TrustedResolveGateResolvedEntry):
            normalized = deepcopy(entry)
            normalized["finding_fingerprint"] = fingerprint
            out.append(normalized)
    return out


def _artifact_mapping(resolved_memory: Mapping[str, Any] | Sequence[Mapping[str, Any]] | None) -> Mapping[str, Any]:
    if isinstance(resolved_memory, Mapping):
        return resolved_memory
    return {}


def _items_from_artifact(resolved_memory: Mapping[str, Any] | Sequence[Mapping[str, Any]] | None) -> list[Mapping[str, Any]]:
    if isinstance(resolved_memory, Mapping):
        raw_items = resolved_memory.get("items") or []
    else:
        raw_items = resolved_memory or []
    if not isinstance(raw_items, list):
        return []
    return [item for item in raw_items if isinstance(item, Mapping)]


def _entries_from_ledger(ledger: Mapping[str, Any] | Sequence[Mapping[str, Any]] | None) -> list[Mapping[str, Any]]:
    if isinstance(ledger, Mapping):
        raw_entries = ledger.get("entries") or []
    else:
        raw_entries = ledger or []
    if not isinstance(raw_entries, list):
        return []
    return [entry for entry in raw_entries if isinstance(entry, Mapping)]


def _scope_from_artifact(artifact: Mapping[str, Any]) -> dict[str, Any]:
    return {
        "repository": str(artifact.get("repository") or ""),
        "pr_number": _safe_int(artifact.get("pr_number"), default=0),
        "base_ref": str(artifact.get("base_ref") or ""),
    }


def _entry_id(item: Mapping[str, Any], fingerprint: str, index: int) -> str:
    thread_id = str(item.get("thread_id") or "unknown-thread").strip() or "unknown-thread"
    return f"resolve-gate:{thread_id}:{fingerprint}:{index}"


def _created_at(item: Mapping[str, Any]) -> str:
    return str(item.get("created_at") or item.get("resolved_at") or "1970-01-01T00:00:00Z")


def _round(item: Mapping[str, Any]) -> int:
    return _safe_int(item.get("round"), default=0)


def _entry_body(item: Mapping[str, Any], state: str, source_schema_version: str) -> dict[str, Any]:
    body = {
        "state": state,
        "reason": item.get("reason"),
        "thread_id": item.get("thread_id"),
        "root_cause_key": item.get("root_cause_key"),
        "path": item.get("path"),
        "line": item.get("line"),
        "source_schema_version": source_schema_version,
    }
    return {key: value for key, value in body.items() if value is not None}


def _resolve_gate_provenance(source_schema_version: str) -> dict[str, Any]:
    return {
        "trusted": True,
        "source_stage": "resolve_gate",
        "source_schema_version": source_schema_version,
        "trust_boundary": "trusted_resolve_gate_artifact",
    }


def _safe_int(value: Any, *, default: int) -> int:
    if isinstance(value, bool):
        return default
    try:
        return int(value)
    except (TypeError, ValueError):
        return default
