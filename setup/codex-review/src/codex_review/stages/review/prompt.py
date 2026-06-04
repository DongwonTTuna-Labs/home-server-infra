"""Stage01 prompt builder."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_text


def include_axis_specific_focus(axis: str) -> str:
    focuses={"correctness":"bugs, edge cases, state transitions","security":"secrets, auth, injection, unsafe trust boundaries","performance":"unbounded work, memory, network, algorithms","test-coverage":"missing tests and regression coverage","domain":"project-specific correctness and product requirements"}
    return focuses.get(axis, "general review")

def include_inspection_evidence_contract(prompt: str) -> str:
    return prompt + "\n\nBefore deciding findings, inspect relevant repo files under pr-head: changed files, nearby implementation, tests, docs, AGENTS.md, and OpenSpec artifacts when present. Return top-level inspection_evidence as a non-empty array. Each inspection_evidence item must include path, purpose, and observation. Each inspection_evidence.path must be an existing file in pr-head, not a directory and not a missing target path. If the issue is a missing file, cite the existing task, spec, proposal, design, or source file that proves the file is required, and put the missing file path in observation or the finding text. Stage01 needs inspection_evidence even when findings is empty."

def include_changed_line_contract(prompt: str, changed_line_map: dict[str, Any]) -> str:
    return prompt + "\n\nOnly emit inline findings with file/line on changed RIGHT-side lines. If repo inspection finds PR-scope risk outside changed RIGHT-side lines, summarize it in finding context/evidence only when it can be anchored to a changed line; otherwise leave it for Stage02 defer_to_issue routing through the finding summary and inspection_evidence. Changed line map:\n" + str(changed_line_map)

def build_axis_prompt(axis: str, pr_context: dict[str, Any], review_context: str, docs_context: str, config: dict[str, Any]) -> str:
    prompt=f"""You are the {axis} reviewer. Focus on {include_axis_specific_focus(axis)}.
Return JSON schema_version review-axis-findings.v1 with axis and findings.
Each finding needs finding_id, severity, file, line, root_cause_key, title, summary, recommendation.
Review the PR against its title/body and any OpenSpec context in the repository docs. Treat OpenSpec proposal, design, tasks, and specs as source of truth. Look for missing implementation, spec mismatch, incomplete tasks, and regression risk.

{docs_context}

{review_context}

## PR context
{pr_context}
"""
    return include_changed_line_contract(include_inspection_evidence_contract(prompt), pr_context.get("changed_line_map", {}))

def write_axis_prompt(axis: str, prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
