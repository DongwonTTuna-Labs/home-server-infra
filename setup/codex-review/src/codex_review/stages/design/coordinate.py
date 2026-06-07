"""Coordinate final design plan."""
from __future__ import annotations
import hashlib, json
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError
from codex_review.model.inspection import (
    collect_existing_evidence_paths,
    render_evidence_citation_hint,
    validate_inspection_evidence,
)


def build_coordinate_prompt(design_context: dict[str, Any], clusters: dict[str, Any], analyses: list[dict[str, Any]]) -> str:
    # The build-plan-prompt handler passes the design inventory as `clusters`;
    # its inspection_evidence paths are already verified to exist in pr-head.
    citation_hint = render_evidence_citation_hint(collect_existing_evidence_paths(clusters, design_context))
    openspec_line = ""
    if design_context.get("openspec_backed"):
        openspec_line = (
            "This is an OpenSpec-backed implementation plan. Treat proposal.md, design.md, "
            "tasks.md, specs/**/*.md, and OpenSpec config included in context as source of truth. "
            "Produce a closed candidate plan with edit_sequence, tests, acceptance_criteria, "
            "allowed_files, openspec_backed=true, and openspec_sources. Approval and fallback routing "
            "belong to design_chief/issue_fallback, not this plan.\n"
        )
    instructions = (
        "Coordinate a candidate design plan. Return design-plan.v1 JSON with edit_sequence and tests.\n"
        "First inspect relevant files in pr-head. Include top-level inspection_evidence as a non-empty "
        "array of objects with path, purpose, and observation for the repo files that informed the plan. "
        "Each inspection_evidence.path must be an existing file in pr-head, not a directory and not a "
        "missing target path. If the issue is a missing file, cite the existing task/spec/design/proposal "
        "file that proves it is required and put the missing file path in observation or edit_sequence.\n"
        "Do not include human-routing fields in this artifact. design_chief design chief decides whether the "
        "candidate is approved_for_fix, needs_human, rejected_plan, or no_fix_needed. For OpenSpec-backed "
        "work, needs_human must be reserved for explicit non-executable blockers; otherwise close the plan "
        "with edit_sequence, tests, and acceptance_criteria.\n"
    )
    return instructions + citation_hint + "\n" + openspec_line + str({"context":design_context,"clusters":clusters,"analyses":analyses})


def validate_design_plan(
    plan: dict[str, Any],
    design_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path | None = None,
) -> dict[str, Any]:
    if "open_questions" in plan:
        raise ValidationError("design design plan does not accept open_questions; use design_chief needs_human routing")
    evidence = validate_inspection_evidence(plan, repo_path, "design design plan")
    out=dict(plan); out["schema_version"]="design-plan.v1"
    out["inspection_evidence"] = evidence
    out.setdefault("edit_sequence", out.get("tasks") or [])
    out.setdefault("tests", [])
    out.setdefault("acceptance_criteria", [])
    out.setdefault("execution_blockers", [])
    open_ctx = design_context.get("openspec_context") or {}
    if out.get("openspec_backed") or design_context.get("openspec_backed"):
        out["openspec_backed"] = True
        out.setdefault("openspec_sources", open_ctx.get("source_summary") or [])
        if not out.get("acceptance_criteria"):
            criteria = []
            for task in out.get("edit_sequence", []):
                criteria.extend(task.get("acceptance_criteria") or [])
            out["acceptance_criteria"] = criteria or ["OpenSpec tasks and referenced specs are satisfied"]
    else:
        out.setdefault("openspec_backed", False)
        out.setdefault("openspec_sources", [])
    if not out.get("edit_sequence") and design_context.get("findings"):
        raise ValidationError("design plan needs edit_sequence for design findings")
    if not out.get("tests") and design_context.get("findings"):
        raise ValidationError("design plan needs tests")
    out["plan_hash"]=compute_design_plan_hash(out)
    return out


def write_design_plan(plan: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, plan, "design-plan.v1")


def compute_design_plan_hash(plan: dict[str, Any]) -> str:
    clean={k:v for k,v in plan.items() if k!="plan_hash"}
    return hashlib.sha256(json.dumps(clean, sort_keys=True, default=str).encode()).hexdigest()[:24]
