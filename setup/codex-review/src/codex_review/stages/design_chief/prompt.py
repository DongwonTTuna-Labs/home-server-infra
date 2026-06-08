"""Design chief prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_text
from codex_review.context.budget import compact_json
from codex_review.model.inspection import (
    collect_existing_evidence_paths,
    render_evidence_citation_hint,
)
from codex_review.stages.design.normalize import render_advisory_memory_context


def include_approval_contract(prompt: str) -> str:
    return prompt + "\nReturn status approved_for_fix, needs_human, rejected_plan, or no_fix_needed. If the plan is OpenSpec-backed, has edit_sequence/tests, and has no execution_blockers, return approved_for_fix. Use needs_human only for secret/live credential needs, unsafe fork mutation, missing OpenSpec source, or other non-executable blockers."

def include_fix_policy_requirements(prompt: str) -> str:
    return prompt + "\nIf approved_for_fix, include fix_policy with allowed_files/allowed_prefixes, forbidden_files."

def include_inspection_evidence_contract(prompt: str) -> str:
    return prompt + "\nBefore deciding status, inspect relevant repo files under pr-head and compare the design plan against OpenSpec/task context when present. Return top-level inspection_evidence as a non-empty array of objects with path, purpose, and observation. Each inspection_evidence.path must be an existing file in pr-head, not a directory and not a missing target path. If the issue is a missing file, cite the existing task/spec/design/proposal file that proves it is required and put the missing file path in observation or decision reason."

def build_design_chief_prompt(
    design_plan: dict[str, Any],
    techlead_decision: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    memory_context: str | None = None,
) -> str:
    prompt=(
        "Review the design plan for safe autofix.\n"
        f"Plan: {compact_json(design_plan)}\n"
        f"Techlead: {compact_json(techlead_decision)}\n"
        f"PR: {compact_json(pr_context)}\n"
        f"Policy: {compact_json(config.get('autofix', {}))}\n"
    )
    prompt += render_advisory_memory_context(memory_context)
    prompt = include_inspection_evidence_contract(include_fix_policy_requirements(include_approval_contract(prompt)))
    # The plan and techlead decision already passed evidence validation, so their
    # paths are a verified existing-file set for the chief to re-cite.
    return prompt + render_evidence_citation_hint(collect_existing_evidence_paths(design_plan, techlead_decision))

def write_design_chief_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
