"""Validate single fix agent result."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError
from codex_review.security.patch_policy import validate_patch_policy, parse_patch_touched_files


def validate_agent_patch(patch_text: str, task: dict[str, Any], policy: dict[str, Any]) -> dict[str, Any]:
    merged={**policy, "allowed_files": task.get("allowed_files") or policy.get("allowed_files")}
    return validate_patch_policy(patch_text, merged, {})


def validate_fix_agent_result(result: dict[str, Any], task: dict[str, Any], policy: dict[str, Any], repo_path: str | Path | None = None) -> dict[str, Any]:
    out=dict(result); out["schema_version"]="fix-dispatch-agent-result.v1"; out.setdefault("task_id", task.get("task_id"))
    from codex_review.patches.fix_edits import ensure_patch_from_edits
    ensure_patch_from_edits(out, repo_path)
    status=out.get("status", "patched")
    if status == "patched":
        patch=out.get("patch") or out.get("patch_text") or ""
        if not patch: raise ValidationError("patched agent result missing patch")
        out["policy_report"]=validate_agent_patch(patch, task, policy)
    elif status not in {"no_safe_fix", "failed"}:
        raise ValidationError(f"invalid agent result status: {status}")
    return out


def validate_no_cross_task_edit(result: dict[str, Any], manifest: dict[str, Any]) -> None:
    tasks={t.get("task_id"): t for t in manifest.get("tasks", [])}
    task=tasks.get(result.get("task_id"))
    if not task: raise ValidationError("result references unknown task")
    touched=parse_patch_touched_files(result.get("patch") or result.get("patch_text") or "")
    allowed=set(task.get("allowed_files") or [])
    for f in touched:
        if f not in allowed and not any(f.startswith(a.rstrip('/') + '/') for a in allowed):
            raise ValidationError(f"agent result edits outside task allowlist: {f}")


def write_validated_agent_result(result: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, result, "fix-dispatch-agent-result.v1")
