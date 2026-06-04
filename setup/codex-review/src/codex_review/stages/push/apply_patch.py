"""Apply merged patch to worktree."""
from __future__ import annotations
import subprocess
from pathlib import Path
from typing import Any
from codex_review.core.errors import ValidationError
from codex_review.security.patch_policy import validate_patch_policy
from .safe_subprocess import sanitized_env


def apply_merged_patch(patch_path: str | Path, repo_path: str | Path) -> dict[str, Any]:
    patch=Path(patch_path).read_text(encoding="utf-8")
    proc=subprocess.run(["git","apply","--index","-"], input=patch, text=True, cwd=Path(repo_path), capture_output=True, env=sanitized_env())
    if proc.returncode!=0: raise ValidationError(f"git apply failed: {proc.stderr.strip()}")
    return {"applied": True, "patch_bytes": len(patch.encode())}


def run_diff_check(repo_path: str | Path) -> None:
    proc=subprocess.run(["git","diff","--quiet","HEAD","--"], cwd=Path(repo_path), env=sanitized_env())
    if proc.returncode==0: raise ValidationError("patch produced no diff")


def collect_applied_diff(repo_path: str | Path) -> str:
    proc=subprocess.run(["git","diff","--binary","HEAD","--"], cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    return proc.stdout


def revalidate_applied_patch(repo_path: str | Path, policy: dict[str, Any]) -> dict[str, Any]:
    return validate_patch_policy(collect_applied_diff(repo_path), policy, {})
