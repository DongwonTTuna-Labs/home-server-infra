"""Push guard validation."""
from __future__ import annotations
import subprocess
from pathlib import Path
from typing import Any
from codex_review.core.errors import PolicyViolation, ValidationError
from codex_review.github.pull_requests import get_current_head_sha
from codex_review.security.provenance import validate_pull_request_target_safety
from codex_review.security.subprocess_env import sanitized_env


def validate_current_head(pr_context: dict[str, Any], merged_fix: dict[str, Any], token: str | None) -> None:
    expected=merged_fix.get("expected_head_sha") or pr_context.get("head_sha")
    current=pr_context.get("current_head_sha") or pr_context.get("head_sha")
    if token and pr_context.get("owner") and pr_context.get("repo") and pr_context.get("pr_number"):
        current=get_current_head_sha(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), token)
    if expected and current and expected != current:
        raise ValidationError(f"head SHA drift: expected {expected}, current {current}")


def validate_push_provenance(pr: dict[str, Any], event: dict[str, Any], policy: dict[str, Any]) -> dict[str, Any]:
    return validate_pull_request_target_safety(event, pr, policy)


def validate_push_target(pr_context: dict[str, Any]) -> None:
    """Block unsafe PR-branch mutation targets.

    The workflow checks out trusted base code and then a separate PR head worktree.
    A content push is only allowed for same-repository PR branches. Fork PRs can be
    reviewed/commented on, but this helper must not push to them from
    pull_request_target.
    """
    same_repo = pr_context.get("same_repo")
    head_repo = pr_context.get("head_repo_full_name")
    base_repo = pr_context.get("base_repo_full_name") or pr_context.get("repository")
    if same_repo is False or (head_repo and base_repo and head_repo != base_repo):
        raise PolicyViolation(f"autofix push is blocked for fork PR head repository: {head_repo}")
    if not pr_context.get("head_ref"):
        raise ValidationError("autofix push requires pr_context.head_ref")
    if not pr_context.get("owner") or not pr_context.get("repo") or not pr_context.get("pr_number"):
        raise ValidationError("autofix push requires owner/repo/pr_number")


def validate_worktree_clean(repo_path: str | Path) -> None:
    proc=subprocess.run(["git","status","--porcelain"], cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    if proc.returncode==0 and proc.stdout.strip(): raise ValidationError("worktree is not clean")


def validate_ready_to_push(merged_fix: dict[str, Any]) -> None:
    if merged_fix.get("status") in {"blocked", "failed"}: raise ValidationError("merged fix is blocked")
    if not (merged_fix.get("patch") or merged_fix.get("patch_text") or merged_fix.get("patch_path")): raise ValidationError("no patch available to push")
