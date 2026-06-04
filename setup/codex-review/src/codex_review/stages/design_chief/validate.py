"""Design chief decision validation.

Stage04 is a quality gate, not a generic human-approval stop. When the design
plan is OpenSpec-backed and executable, the validator normalizes conservative
model output to ``approved_for_fix`` so fix_dispatch can implement the PR to LGTM.
"""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError
from codex_review.model.inspection import validate_inspection_evidence

VALID_STATUSES={"approved_for_fix","needs_human","rejected_plan","no_fix_needed"}
NON_EXECUTABLE_BLOCKERS={
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


def validate_fix_policy(policy: dict[str, Any], design_plan: dict[str, Any], config: dict[str, Any]) -> None:
    if not isinstance(policy, dict): raise ValidationError("fix_policy must be an object")
    allowed=policy.get("allowed_files") or policy.get("allowed_prefixes") or config.get("autofix", {}).get("allowed_prefixes")
    if not allowed: raise ValidationError("fix_policy requires allowed_files or allowed_prefixes")


def validate_task_hints(task_hints: list[dict[str, Any]], design_plan: dict[str, Any]) -> None:
    if task_hints is None: return
    ids={s.get("task_id") or s.get("id") for s in design_plan.get("edit_sequence", [])}
    for hint in task_hints:
        if hint.get("task_id") and ids and hint.get("task_id") not in ids:
            raise ValidationError(f"task_hint references unknown task: {hint.get('task_id')}")


def _normalize_blocker(value: Any) -> str:
    return str(value or "").strip().lower().replace(" ", "_").replace("-", "_")


def _execution_blockers(design_plan: dict[str, Any]) -> list[str]:
    raw = design_plan.get("execution_blockers") or []
    if isinstance(raw, str):
        raw = [raw]
    return [str(item) for item in raw if str(item).strip()]


def _has_non_executable_blocker(design_plan: dict[str, Any]) -> bool:
    blockers = _execution_blockers(design_plan)
    return bool(blockers)


def _is_executable_openspec_plan(design_plan: dict[str, Any]) -> bool:
    if not design_plan.get("openspec_backed"):
        return False
    if _has_non_executable_blocker(design_plan):
        return False
    return bool(design_plan.get("edit_sequence") and design_plan.get("tests") and design_plan.get("acceptance_criteria"))


def block_approval_when_human_review_required(decision: dict[str, Any], design_plan: dict[str, Any]) -> None:
    # Generic human-review flags are not enough to stop an otherwise executable
    # OpenSpec-backed plan. Explicit non-executable blockers are handled above.
    if decision.get("status") == "approved_for_fix" and design_plan.get("requires_human_review") and not design_plan.get("openspec_backed"):
        raise ValidationError("cannot approve fix when design plan requires human review")


def promote_openspec_backed_plan(decision: dict[str, Any], design_plan: dict[str, Any]) -> dict[str, Any]:
    if decision.get("status") == "approved_for_fix":
        return decision
    if not _is_executable_openspec_plan(design_plan):
        return decision
    out = dict(decision)
    out["status"] = "approved_for_fix"
    out["normalized_from"] = decision.get("status") or "missing_status"
    out["reason"] = "OpenSpec-backed design plan is executable; conservative stop was normalized to approved_for_fix."
    return out


def validate_chief_decision(
    decision: dict[str, Any],
    design_plan: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path | None = None,
) -> dict[str, Any]:
    evidence = validate_inspection_evidence(decision, repo_path, "design_chief design chief decision")
    out=promote_openspec_backed_plan(dict(decision), design_plan)
    out["schema_version"]="design-chief-decision.v1"
    out["inspection_evidence"] = evidence
    status=out.get("status")
    if status not in VALID_STATUSES: raise ValidationError(f"invalid chief status: {status}")
    block_approval_when_human_review_required(out, design_plan)
    if status == "approved_for_fix":
        out.setdefault("fix_policy", {})
        # Merge config defaults while letting chief constrain more tightly.
        merged={**config.get("autofix", {}), **out.get("fix_policy", {})}
        out["fix_policy"]=merged
        validate_fix_policy(merged, design_plan, config)
        validate_task_hints(out.get("task_hints", []), design_plan)
    elif status == "needs_human" and design_plan.get("openspec_backed") and not _has_non_executable_blocker(design_plan):
        raise ValidationError("OpenSpec-backed needs_human requires an explicit non-executable execution_blocker")
    return out


def write_validated_chief_decision(decision: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, decision, "design-chief-decision.v1")
