"""Commit helper for trusted push."""
from __future__ import annotations
import subprocess
from pathlib import Path
from typing import Any
from codex_review.patches.commit_plan import normalize_commit_plan
from codex_review.core.errors import ValidationError
from codex_review.security.patch_policy import validate_patch_policy
from .safe_subprocess import sanitized_env


def commit_plan_from_artifacts(merged_fix: dict[str, Any], validation_result: dict[str, Any] | None = None, patch_text: str | None = None) -> list[dict[str, Any]]:
    semantic = (validation_result or {}).get("semantic_safety")
    raw_plan = None
    if isinstance(semantic, dict):
        raw_plan = semantic.get("commit_plan")
    if raw_plan is None:
        raw_plan = merged_fix.get("commit_plan")
    return normalize_commit_plan(raw_plan, patch_text)


def build_commit_message(merged_fix: dict[str, Any], design_plan_hash: str, old_head_sha: str, entry: dict[str, Any] | None = None) -> str:
    plan = normalize_commit_plan([entry] if entry is not None else merged_fix.get("commit_plan"))
    selected = plan[0]
    body = str(selected.get("body") or "").strip()
    parts = [selected["subject"], ""]
    if body:
        parts.extend([body, ""])
    parts.extend(
        [
            f"Design-plan-hash: {design_plan_hash}",
            f"Previous-head-sha: {old_head_sha}",
            "Marker: codex-review:autofix",
            "",
        ]
    )
    return "\n".join(parts)


def configure_git_author(policy: dict[str, Any]) -> None:
    name=policy.get("git_author_name", "Codex Review Bot"); email=policy.get("git_author_email", "codex-review@example.invalid")
    subprocess.run(["git","config","user.name",name], check=True, env=sanitized_env())
    subprocess.run(["git","config","user.email",email], check=True, env=sanitized_env())


def create_commit(repo_path: str | Path, message: str, paths: list[str] | None = None) -> str:
    if paths:
        subprocess.run(["git", "add", "--", *paths], cwd=Path(repo_path), check=True, env=sanitized_env())
    else:
        subprocess.run(["git", "add", "-A"], cwd=Path(repo_path), check=True, env=sanitized_env())
    diff_proc = subprocess.run(["git", "diff", "--cached", "--quiet", "--"], cwd=Path(repo_path), env=sanitized_env())
    if diff_proc.returncode == 0:
        raise ValidationError("commit plan entry produced no staged diff")
    proc = subprocess.run(["git", "commit", "--no-verify", "-F", "-"], input=message, cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    if proc.returncode != 0:
        raise ValidationError(f"git commit failed: {proc.stderr.strip()}")
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=Path(repo_path), text=True, env=sanitized_env()).strip()
    return sha


def create_commits_from_plan(repo_path: str | Path, commit_plan: list[dict[str, Any]], design_plan_hash: str, old_head_sha: str, merged_fix: dict[str, Any]) -> list[str]:
    repo = Path(repo_path)
    subprocess.run(["git", "reset"], cwd=repo, check=True, stdout=subprocess.DEVNULL, env=sanitized_env())
    shas: list[str] = []
    for entry in commit_plan:
        message = build_commit_message(merged_fix, design_plan_hash, old_head_sha, entry)
        shas.append(create_commit(repo, message, list(entry.get("paths") or [])))
    status = subprocess.run(["git", "status", "--porcelain"], cwd=repo, capture_output=True, text=True, env=sanitized_env())
    if status.stdout.strip():
        raise ValidationError("commit plan did not commit all applied changes")
    return shas


def validate_commit_diff(repo_path: str | Path, commit_sha: str, policy: dict[str, Any]) -> dict[str, Any]:
    patch=subprocess.check_output(["git","show","--format=","--binary",commit_sha], cwd=Path(repo_path), text=True, env=sanitized_env())
    return validate_patch_policy(patch, policy, {})
