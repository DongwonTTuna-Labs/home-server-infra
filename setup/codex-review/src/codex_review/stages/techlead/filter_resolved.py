"""Filter combined findings against previously-resolved threads (anti re-flag).

Stops the loop from re-raising a finding that an earlier run already resolved with a
reason. Runs as a dedicated step in the techlead job, before the techlead prompt is
built, so re-flagged findings never reach triage. Suppressions are recorded (never a
silent drop).
"""
from __future__ import annotations

from typing import Any

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


def _decide(finding: dict[str, Any], mem: dict[str, Any], changed_line_map: dict[str, Any] | None, suppress_states: set[str], recheck_changed: bool) -> tuple[bool, str | None]:
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


def filter_findings_against_resolved(
    combined: dict[str, Any],
    resolved_memory: dict[str, Any] | list[dict[str, Any]] | None,
    changed_line_map: dict[str, Any] | None,
    config: dict[str, Any] | None,
) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    review = (config or {}).get("review", {}) or {}
    suppress_states = set(review.get("suppress_resolved_states") or _DEFAULT_SUPPRESS_STATES)
    recheck_changed = bool(review.get("resolved_by_code_recheck_changed", True))

    items = resolved_memory.get("items", []) if isinstance(resolved_memory, dict) else (resolved_memory or [])
    by_key: dict[str, dict[str, Any]] = {}
    by_loc: dict[tuple[str, int], dict[str, Any]] = {}
    for m in items:
        if m.get("root_cause_key"):
            by_key.setdefault(str(m["root_cause_key"]), m)
        line = _as_int(m.get("line"))
        if m.get("path") and line is not None:
            by_loc.setdefault((str(m["path"]), line), m)

    kept: list[dict[str, Any]] = []
    suppressed: list[dict[str, Any]] = []
    annotated: list[str] = []
    for f in combined.get("findings", []) or []:
        line = _as_int(f.get("line"))
        mem = by_key.get(str(f.get("root_cause_key")))
        if mem is None and f.get("file") and line is not None:
            mem = by_loc.get((str(f.get("file")), line))
        if mem is None:
            kept.append(f)
            continue
        suppress, state = _decide(f, mem, changed_line_map, suppress_states, recheck_changed)
        if suppress:
            suppressed.append({**f, "previously_resolved_as": state, "resolution_reason": mem.get("reason"), "resolved_thread_id": mem.get("thread_id")})
        else:
            nf = dict(f)
            nf["previously_resolved_as"] = state
            nf["resolution_reason"] = mem.get("reason")
            kept.append(nf)
            if f.get("finding_id"):
                annotated.append(f["finding_id"])

    out = dict(combined)
    out["findings"] = kept
    out["finding_count"] = len(kept)
    out["suppressed_resolved"] = suppressed
    out["suppressed_resolved_count"] = len(suppressed)
    out["resolved_annotated_ids"] = annotated
    return out, suppressed
