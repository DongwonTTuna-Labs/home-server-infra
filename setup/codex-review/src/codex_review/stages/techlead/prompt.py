"""Stage02 techlead prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_text
from codex_review.context.budget import compact_json


def include_decision_action_contract(prompt: str) -> str:
    return prompt + "\nActions: publish_and_fix_now, summary_only_fix_now, defer_to_issue, deny_false_positive, needs_human, needs_design, publish_only, drop_duplicate. Cover every finding_id exactly once. Do not use needs_human as a generic uncertainty escape. For implementable OpenSpec-backed findings, choose needs_design or publish_and_fix_now. Use defer_to_issue for work outside this PR, missing OpenSpec source, fork push limits, or non-executable blockers."

def include_design_required_contract(prompt: str) -> str:
    return prompt + "\nSet needs_design=true for findings that require coordinated edits or autofix."

def include_inspection_evidence_contract(prompt: str) -> str:
    return prompt + "\nBefore routing, inspect relevant repo files under pr-head instead of relying only on the Stage01 summaries. Return top-level inspection_evidence as a non-empty array. Each item must include path, purpose, and observation. Each inspection_evidence.path must be an existing file in pr-head, not a directory and not a missing target path. If the issue is a missing file, cite the existing task, spec, proposal, design, or source file that proves the file is required, and put the missing file path in observation or routing reason."

def build_techlead_prompt(combined_findings: dict[str, Any], pr_context: dict[str, Any], review_context: str, docs_context: str, config: dict[str, Any]) -> str:
    ctx_budget = (config or {}).get("context", {}) or {}
    findings_json = compact_json(combined_findings, max_tokens=int(ctx_budget.get("findings_tokens", 12000)))
    pr_context_json = compact_json(pr_context)
    prompt=f"""You are the Codex Review tech lead. Reduce axis findings to actionable decisions.
Return JSON schema_version techlead-decision.v1.

{docs_context}

{review_context}

Combined findings:
{findings_json}

PR context:
{pr_context_json}
"""
    return include_inspection_evidence_contract(include_design_required_contract(include_decision_action_contract(prompt)))

def write_techlead_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
