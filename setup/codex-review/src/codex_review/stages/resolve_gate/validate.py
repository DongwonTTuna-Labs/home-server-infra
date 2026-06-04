"""Stage00 lifecycle-result validation."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.constants import LIFECYCLE_STATES, TERMINAL_LIFECYCLE_STATES
from codex_review.core.errors import ValidationError
from codex_review.core.schema import validate_enum


def _decisions(result: dict[str, Any]) -> list[dict[str, Any]]:
    return result.get("decisions") or result.get("items") or []


def validate_lifecycle_state(decision: dict[str, Any]) -> None:
    validate_enum(decision.get("state"), LIFECYCLE_STATES, "lifecycle state")


def validate_terminal_evidence(decision: dict[str, Any], inventory_item: dict[str, Any]) -> None:
    if decision.get("state") in TERMINAL_LIFECYCLE_STATES:
        if not decision.get("evidence") and not decision.get("evidence_hash"):
            raise ValidationError(f"terminal lifecycle decision needs evidence: {decision.get('thread_id')}")
        if inventory_item.get("forced_needs_human"):
            raise ValidationError(f"forced_needs_human thread cannot be terminal: {decision.get('thread_id')}")


def validate_issue_request(decision: dict[str, Any]) -> None:
    if decision.get("state") == "defer_to_issue":
        issue = decision.get("issue") or decision.get("issue_request")
        if not isinstance(issue, dict) or not (issue.get("title") and (issue.get("body") or issue.get("root_cause_key"))):
            raise ValidationError("defer_to_issue requires issue_request/title/body")


def prevent_current_head_resolution(result: dict[str, Any], inventory: dict[str, Any]) -> None:
    by_id={i.get("thread_id"): i for i in inventory.get("items", [])}
    for d in _decisions(result):
        item=by_id.get(d.get("thread_id"))
        if item and item.get("is_current_head") and d.get("state") in TERMINAL_LIFECYCLE_STATES:
            raise ValidationError(f"current head thread cannot be resolved: {d.get('thread_id')}")


def validate_lifecycle_result(result: dict[str, Any], inventory: dict[str, Any]) -> dict[str, Any]:
    if result.get("schema_version") and result.get("schema_version") != "resolve-gate-lifecycle-result.v1":
        raise ValidationError("wrong lifecycle result schema_version")
    decisions=_decisions(result)
    if not isinstance(decisions, list):
        raise ValidationError("lifecycle result decisions must be a list")
    expected=[i.get("thread_id") for i in inventory.get("items", [])]
    got=[d.get("thread_id") for d in decisions]
    if len(got) != len(set(got)):
        raise ValidationError("duplicate thread_id in lifecycle result")
    if set(got) != set(expected):
        missing=set(expected)-set(got); unknown=set(got)-set(expected)
        raise ValidationError(f"lifecycle result thread coverage mismatch missing={sorted(missing)} unknown={sorted(unknown)}")
    by_id={i.get("thread_id"): i for i in inventory.get("items", [])}
    for d in decisions:
        validate_lifecycle_state(d)
        item=by_id[d.get("thread_id")]
        validate_terminal_evidence(d, item)
        validate_issue_request(d)
        if item.get("forced_needs_human") and d.get("state") != "needs_human":
            raise ValidationError(f"forced_needs_human thread must be needs_human: {d.get('thread_id')}")
    prevent_current_head_resolution(result, inventory)
    out=dict(result); out["schema_version"]="resolve-gate-lifecycle-result.v1"; out["decisions"]=decisions
    return out


def write_validated_lifecycle_result(result: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, result, "resolve-gate-lifecycle-result.v1")
