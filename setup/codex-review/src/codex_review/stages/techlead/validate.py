"""Stage02 techlead decision validation.

Stage02 reduces reviewer findings into execution routing. For OpenSpec-backed
work, generic ``needs_human`` is not a valid escape hatch: implementable findings
must proceed to design/fix, while physically non-executable work must be marked
with an explicit blocker or deferred to issue fallback.
"""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.constants import TECHLEAD_ACTIONS, SEVERITIES
from codex_review.core.errors import ValidationError
from codex_review.model.inspection import validate_inspection_evidence
from codex_review.core.schema import validate_enum

NON_EXECUTABLE_BLOCKERS = {
    "missing_openspec_spec",
    "unresolved_openspec_source",
    "fork_pr_push_blocked",
    "secret_required",
    "live_credential_required",
    "external_system_required",
    "legal_or_security_blocker",
    "out_of_scope",
    "no_mutation_permission",
}


def validate_decision_action(item: dict[str, Any]) -> None:
    validate_enum(item.get("action"), TECHLEAD_ACTIONS, "techlead action")


def validate_scope_and_severity(item: dict[str, Any]) -> None:
    if item.get("severity"): validate_enum(item.get("severity"), SEVERITIES, "severity")


def validate_design_required(decision: dict[str, Any]) -> None:
    if decision.get("needs_design") and not any((i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"}) for i in decision.get("decisions", [])):
        raise ValidationError("needs_design=true requires at least one design/fix action")


def _blocker_value(item: dict[str, Any]) -> str | None:
    for key in ("blocker_type", "execution_blocker", "non_executable_reason", "fallback_reason"):
        value = item.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    blockers = item.get("execution_blockers")
    if isinstance(blockers, list) and blockers:
        return str(blockers[0])
    return None


def has_non_executable_blocker(item: dict[str, Any]) -> bool:
    blocker = _blocker_value(item)
    if not blocker:
        return False
    normalized = blocker.lower().replace(" ", "_").replace("-", "_")
    return normalized in NON_EXECUTABLE_BLOCKERS


def normalize_generic_needs_human(item: dict[str, Any]) -> dict[str, Any]:
    if item.get("action") != "needs_human":
        return item
    if has_non_executable_blocker(item):
        return item
    out = dict(item)
    out["action"] = "needs_design"
    out["normalized_from"] = "needs_human"
    out["normalization_reason"] = "generic needs_human is not allowed for OpenSpec-driven implementable work; routed to design/fix"
    return out


def annotate_semantic_risk(item: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    """Record semantic risk hints without hard-blocking implementation routing."""
    text=" ".join(str(item.get(k,"")) for k in ["summary","risk","reason"] ).lower()
    dangerous=[k.lower() for k in config.get("autofix", {}).get("dangerous_keywords", [])]
    hits=sorted({keyword for keyword in dangerous if keyword and keyword in text})
    if not hits:
        return item
    out=dict(item)
    out.setdefault("semantic_risk_hints", hits)
    return out


def validate_techlead_decision(
    decision: dict[str, Any],
    combined_findings: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path | None = None,
) -> dict[str, Any]:
    evidence = validate_inspection_evidence(decision, repo_path, "techlead techlead decision")
    raw_items=decision.get("decisions") or decision.get("items") or []
    if not isinstance(raw_items, list): raise ValidationError("techlead decisions must be a list")
    expected={f.get("finding_id") or f.get("id") for f in combined_findings.get("findings", [])}
    got=[i.get("finding_id") for i in raw_items]
    if len(got)!=len(set(got)): raise ValidationError("duplicate finding_id in techlead decision")
    if set(got)!=expected:
        raise ValidationError(f"techlead decision must cover all findings exactly once missing={sorted(expected-set(got))} unknown={sorted(set(got)-expected)}")
    items=[]
    for raw in raw_items:
        item = annotate_semantic_risk(normalize_generic_needs_human(dict(raw)), config)
        validate_decision_action(item); validate_scope_and_severity(item)
        items.append(item)
    out=dict(decision); out["schema_version"]="techlead-decision.v1"; out["decisions"]=items; out["inspection_evidence"]=evidence
    out["needs_design"] = bool(any(i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"} for i in items))
    if any(i.get("action") == "needs_human" for i in items):
        out["status"] = "needs_human"
    elif out["needs_design"]:
        out["status"] = "needs_design"
    elif items:
        out["status"] = "ready"
    else:
        out["status"] = "lgtm"
    validate_design_required(out)
    return out


def write_validated_techlead_decision(decision: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, decision, "techlead-decision.v1")
