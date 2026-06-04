"""Fix agent prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_text


def include_patch_output_contract(prompt: str) -> str:
    return prompt + (
        "\nReturn JSON schema_version fix-dispatch-agent-result.v1 with an `edits` array of "
        "{path, old_str, new_str} search/replace objects — NOT a unified diff. Rules: old_str "
        "must appear EXACTLY ONCE in the target file, so include enough surrounding context to be "
        "unique; to create a new file use an empty old_str and put the full file content in new_str. "
        "To DELETE a file, list its path in a top-level `deletions` array (use [] when none). "
        "Only touch allowed files. Do not commit, push, comment, or call GitHub APIs."
    )


def include_no_safe_fix_contract(prompt: str) -> str:
    return prompt + "\nFor OpenSpec-backed tasks, do not use no_safe_fix for uncertainty or conservatism. Use no_safe_fix only for a concrete mechanical blocker such as allowed_files impossibility, missing source file context that prevents a patch, or a policy conflict that issue_fallback must track."


def build_fix_agent_prompt(task: dict[str, Any], design_plan: dict[str, Any], chief_decision: dict[str, Any], source_context: dict[str, Any] | str, config: dict[str, Any]) -> str:
    prompt=f"""You are implementing an approved Codex Review fix task.

This is an OpenSpec-authoritative implementation loop when openspec_backed is true. Treat the PR title/body plus proposal.md, design.md, tasks.md, specs/**/*.md, and OpenSpec config in the source context as the source of truth. The task is already approved for implementation; your job is to produce the patch that moves the PR toward LGTM.

Fix task {task.get('task_id')}: {task.get('summary')}
Allowed files: {task.get('allowed_files')}
Acceptance criteria: {task.get('acceptance_criteria')}
Required tests: {task.get('tests')}
OpenSpec sources: {task.get('openspec_sources')}

Design: {design_plan}
Chief decision: {chief_decision}
Source context: {source_context}
"""
    return include_no_safe_fix_contract(include_patch_output_contract(prompt))


def write_fix_agent_prompt(task_id: str, prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
