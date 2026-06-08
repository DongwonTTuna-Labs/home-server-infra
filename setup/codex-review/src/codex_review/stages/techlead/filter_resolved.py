"""Filter combined findings against previously-resolved threads (anti re-flag).

Stops the loop from re-raising a finding that an earlier run already resolved with a
reason. Runs as a dedicated step in the techlead job, before the techlead prompt is
built, so re-flagged findings never reach triage. Suppressions are recorded (never a
silent drop).
"""
from __future__ import annotations

from typing import Any, Mapping

from codex_review.memory.resolved import (
    exact_finding_fingerprint,
    resolve_gate_resolved_memory_as_ledger,
    resolved_findings_for_suppression,
)
from codex_review.memory.types import SCHEMA_VERSION as REVIEW_MEMORY_SCHEMA_VERSION

# Default safe-first policy: states whose resolution means "do not raise again"
# regardless of subsequent code change. resolved_by_code is handled separately
# (re-allowed if the resolved location changed since resolution).
_DEFAULT_SUPPRESS_STATES = ["false_positive", "stale_obsolete", "duplicate_of_issue", "defer_to_issue"]


def _changed_lines_for(path: Any, changed_line_map: dict[str, Any] | None) -> set[int]:
    values = (changed_line_map or {}).get(str(path)) or []
    out: set[int] = set()
    for value in values if isinstance(values, (list, set, tuple)) else []:
        try:
            out.add(int(value))
        except (TypeError, ValueError):
            continue
    return out


def _as_int(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _decide(
    finding: dict[str, Any],
    mem: dict[str, Any],
    changed_line_map: dict[str, Any] | None,
    suppress_states: set[str],
    recheck_changed: bool,
) -> tuple[bool, str | None]:
    """Return (suppress, state). suppress=False means keep (possibly annotated)."""
    state = mem.get("state")
    if state in suppress_states:
        return True, state
    if state == "resolved_by_code":
        if not recheck_changed:
            return True, state
        path = finding.get("file") or mem.get("path")
        line = _as_int(finding.get("line")) or _as_int(mem.get("line"))
        # If the resolved location is among this PR's changed lines, the fix may have
        # been re-broken -> allow re-raise. Otherwise the prior resolution stands.
        if line is not None and line in _changed_lines_for(path, changed_line_map):
            return False, state
        return True, state
    # Unknown/missing state (e.g. resolved before this marker shipped): annotate, don't drop.
    return False, state


def _is_review_memory_ledger(resolved_memory: Any) -> bool:
    return (
        isinstance(resolved_memory, Mapping)
        and resolved_memory.get("schema_version") == REVIEW_MEMORY_SCHEMA_VERSION
        and isinstance(resolved_memory.get("entries"), list)
    )


def _legacy_items(resolved_memory: dict[str, Any] | list[dict[str, Any]] | None) -> list[dict[str, Any]]:
    if _is_review_memory_ledger(resolved_memory):
        return []
    raw_items = resolved_memory.get("items", []) if isinstance(resolved_memory, dict) else (resolved_memory or [])
    if not isinstance(raw_items, list):
        return []
    return [dict(item) for item in raw_items if isinstance(item, Mapping)]


def _trusted_suppression_entries(
    resolved_memory: dict[str, Any] | list[dict[str, Any]] | None,
    suppress_states: set[str],
) -> list[dict[str, Any]]:
    if _is_review_memory_ledger(resolved_memory):
        return resolved_findings_for_suppression(resolved_memory)
    if isinstance(resolved_memory, dict) and "items" in resolved_memory:
        ledger = resolve_gate_resolved_memory_as_ledger(resolved_memory, suppress_states=suppress_states)
        return resolved_findings_for_suppression(ledger)
    if isinstance(resolved_memory, list):
        ledger = resolve_gate_resolved_memory_as_ledger(resolved_memory, suppress_states=suppress_states)
        return resolved_findings_for_suppression(ledger)
    return []


def _resolved_entry_as_memory(entry: Mapping[str, Any]) -> dict[str, Any]:
    body = entry.get("body") if isinstance(entry.get("body"), dict) else {}
    return {
        "state": body.get("state"),
        "reason": body.get("reason"),
        "thread_id": body.get("thread_id"),
        "path": body.get("path"),
        "line": body.get("line"),
        "finding_fingerprint": entry.get("finding_fingerprint"),
    }


def _index_trusted_entries(entries: list[dict[str, Any]]) -> dict[str, dict[str, Any]]:
    by_fingerprint: dict[str, dict[str, Any]] = {}
    for entry in entries:
        fingerprint = exact_finding_fingerprint(entry)
        if fingerprint:
            by_fingerprint.setdefault(fingerprint, entry)
    return by_fingerprint


def _index_legacy_items(items: list[dict[str, Any]]) -> tuple[dict[str, dict[str, Any]], dict[tuple[str, int], dict[str, Any]]]:
    by_key: dict[str, dict[str, Any]] = {}
    by_loc: dict[tuple[str, int], dict[str, Any]] = {}
    for resolved_item in items:
        if resolved_item.get("root_cause_key"):
            by_key.setdefault(str(resolved_item["root_cause_key"]), resolved_item)
        line = _as_int(resolved_item.get("line"))
        if resolved_item.get("path") and line is not None:
            by_loc.setdefault((str(resolved_item["path"]), line), resolved_item)
    return by_key, by_loc


def _suppressed_finding(finding: dict[str, Any], resolved_item: dict[str, Any], state: str | None) -> dict[str, Any]:
    return {
        **finding,
        "previously_resolved_as": state,
        "resolution_reason": resolved_item.get("reason"),
        "resolved_thread_id": resolved_item.get("thread_id"),
    }


def _annotated_finding(finding: dict[str, Any], resolved_item: dict[str, Any], state: str | None) -> dict[str, Any]:
    annotated = dict(finding)
    annotated["previously_resolved_as"] = state
    annotated["resolution_reason"] = resolved_item.get("reason")
    return annotated


def filter_findings_against_resolved(
    combined: dict[str, Any],
    resolved_memory: dict[str, Any] | list[dict[str, Any]] | None,
    changed_line_map: dict[str, Any] | None,
    config: dict[str, Any] | None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    review = (config or {}).get("review", {}) or {}
    suppress_states = set(review.get("suppress_resolved_states") or _DEFAULT_SUPPRESS_STATES)
    recheck_changed = bool(review.get("resolved_by_code_recheck_changed", True))

    trusted_by_fingerprint = _index_trusted_entries(_trusted_suppression_entries(resolved_memory, suppress_states))
    by_key, by_loc = _index_legacy_items(_legacy_items(resolved_memory))

    kept: list[dict[str, Any]] = []
    suppressed: list[dict[str, Any]] = []
    annotated: list[str] = []
    for finding in combined.get("findings", []) or []:
        fingerprint = exact_finding_fingerprint(finding)
        trusted_entry = trusted_by_fingerprint.get(fingerprint) if fingerprint else None
        if trusted_entry is not None:
            resolved_item = _resolved_entry_as_memory(trusted_entry)
            suppress, state = _decide(finding, resolved_item, changed_line_map, suppress_states, recheck_changed)
            if suppress:
                suppressed.append(_suppressed_finding(finding, resolved_item, state))
            else:
                kept.append(_annotated_finding(finding, resolved_item, state))
                if finding.get("finding_id"):
                    annotated.append(finding["finding_id"])
            continue

        line = _as_int(finding.get("line"))
        resolved_item = by_key.get(str(finding.get("root_cause_key"))) if finding.get("root_cause_key") else None
        if resolved_item is None and finding.get("file") and line is not None:
            resolved_item = by_loc.get((str(finding.get("file")), line))
        if resolved_item is None:
            kept.append(finding)
            continue
        suppress, state = _decide(finding, resolved_item, changed_line_map, suppress_states, recheck_changed)
        if suppress:
            suppressed.append(_suppressed_finding(finding, resolved_item, state))
        else:
            kept.append(_annotated_finding(finding, resolved_item, state))
            if finding.get("finding_id"):
                annotated.append(finding["finding_id"])

    out = dict(combined)
    out["findings"] = kept
    out["finding_count"] = len(kept)
    out["suppressed_resolved"] = suppressed
    out["suppressed_resolved_count"] = len(suppressed)
    out["resolved_annotated_ids"] = annotated
    return out, suppressed
