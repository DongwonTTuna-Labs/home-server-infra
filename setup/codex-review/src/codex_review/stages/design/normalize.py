"""Normalize techlead findings for design."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError
from codex_review.model.inspection import (
    collect_existing_evidence_paths,
    render_evidence_citation_hint,
    validate_inspection_evidence,
)


def build_normalize_prompt(design_context: dict[str, Any]) -> str:
    return (
        "Normalize the following design-relevant findings into invariant-oriented items. "
        "First inspect relevant files in pr-head and include top-level inspection_evidence "
        "items with path, purpose, and observation. Each inspection_evidence.path must be an "
        "existing file in pr-head, not a directory and not a missing target path. If the issue "
        "is a missing file, cite the existing task/spec/design/proposal file that proves it is "
        "required and put the missing file path in observation. Return design-inventory.v1 JSON.\n"
        + render_evidence_citation_hint(collect_existing_evidence_paths(design_context))
        + "\n"
        + str(design_context)
    )


def validate_design_inventory(
    inventory: dict[str, Any],
    techlead_decision: dict[str, Any],
    repo_path: str | Path | None = None,
) -> dict[str, Any]:
    evidence = validate_inspection_evidence(inventory, repo_path, "design design inventory")
    items=inventory.get("items") or inventory.get("findings") or []
    expected={i.get("finding_id") for i in techlead_decision.get("decisions", []) if i.get("action") in {"needs_design","publish_and_fix_now","summary_only_fix_now"}}
    got={i.get("finding_id") for i in items}
    if expected and got != expected:
        raise ValidationError(f"design inventory coverage mismatch missing={sorted(expected-got)} unknown={sorted(got-expected)}")
    out=dict(inventory); out["schema_version"]="design-inventory.v1"; out["items"]=items; out["item_count"]=len(items); out["inspection_evidence"]=evidence
    return out


def write_design_inventory(inventory: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, inventory, "design-inventory.v1")


def summarize_design_inventory(inventory: dict[str, Any]) -> str:
    return f"Design inventory items: {len(inventory.get('items', []))}"
