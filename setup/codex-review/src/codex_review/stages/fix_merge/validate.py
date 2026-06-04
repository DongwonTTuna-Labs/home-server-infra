"""Validate merged fix."""
from __future__ import annotations
import subprocess
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError
from codex_review.security.patch_policy import validate_patch_policy
from codex_review.security.subprocess_env import sanitized_env


def validate_merged_patch_applies(patch_text: str, repo_path: str | Path) -> None:
    if repo_path and (Path(repo_path)/".git").exists():
        proc=subprocess.run(["git","apply","--check","-"], input=patch_text, text=True, cwd=Path(repo_path), capture_output=True, env=sanitized_env())
        if proc.returncode!=0: raise ValidationError(f"merged patch does not apply: {proc.stderr.strip()}")


def validate_merged_patch_policy(patch_text: str, chief_decision: dict[str, Any], policy: dict[str, Any]) -> dict[str, Any]:
    merged={**policy, **chief_decision.get("fix_policy", {})}
    return validate_patch_policy(patch_text, merged, {})


def validate_merged_fix(merged_fix: dict[str, Any], premerge_report: dict[str, Any], chief_decision: dict[str, Any], policy: dict[str, Any], repo_path: str | Path | None = None) -> dict[str, Any]:
    out=dict(merged_fix); out["schema_version"]="fix-merge-merged-fix.v1"
    patch=out.get("patch") or out.get("patch_text") or ""
    status=out.get("status")
    if status in {"no_fix", "blocked"} and not patch:
        out.setdefault("premerge_clean", premerge_report.get("clean"))
        return out
    if not patch: raise ValidationError("merged fix missing patch")
    out["policy_report"]=validate_merged_patch_policy(patch, chief_decision, policy)
    if repo_path:
        validate_merged_patch_applies(patch, repo_path)
    out.setdefault("premerge_clean", premerge_report.get("clean"))
    out.setdefault("status", "ready")
    return out


def write_validated_merged_fix(merged_fix: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, merged_fix, "fix-merge-merged-fix.v1")
