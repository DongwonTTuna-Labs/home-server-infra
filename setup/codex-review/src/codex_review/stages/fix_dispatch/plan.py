"""Plan fix tasks from approved design."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError


def _task_files(task: dict[str, Any]) -> set[str]:
    return set(task.get("allowed_files") or task.get("files") or [])


def merge_tasks_touching_same_files(tasks: list[dict[str, Any]]) -> list[dict[str, Any]]:
    groups: list[dict[str, Any]] = []
    for task in tasks:
        files=_task_files(task)
        merged=False
        for group in groups:
            if files & set(group.get("allowed_files", [])):
                group["allowed_files"]=sorted(set(group.get("allowed_files", [])) | files)
                group["steps"].extend(task.get("steps", [task.get("summary")]))
                group["tests"]=sorted(set(group.get("tests", [])) | set(task.get("tests", [])))
                group["acceptance_criteria"].extend(c for c in task.get("acceptance_criteria", []) if c not in group["acceptance_criteria"])
                group["source_finding_ids"].extend(i for i in task.get("source_finding_ids", []) if i not in group["source_finding_ids"])
                merged=True
                break
        if not merged:
            groups.append(
                {
                    "task_id": task.get("task_id") or task.get("id") or f"fix-{len(groups)+1}",
                    "summary": task.get("summary", "Apply design step"),
                    "allowed_files": sorted(files),
                    "steps": task.get("steps", [task.get("summary")]),
                    "tests": list(task.get("tests", [])),
                    "acceptance_criteria": list(task.get("acceptance_criteria", [])),
                    "source_finding_ids": list(task.get("source_finding_ids", [])),
                    "openspec_sources": list(task.get("openspec_sources", [])),
                    "openspec_backed": bool(task.get("openspec_backed")),
                }
            )
    return groups


def _ensure_unique_task_ids(tasks: list[dict[str, Any]]) -> list[dict[str, Any]]:
    """Make task_ids unique deterministically.

    The design model supplies task_id/id values that can collide; rather than
    hard-failing the whole fix stage, disambiguate collisions by suffixing the
    second and later occurrences (fix-1, fix-1-2, fix-1-3, ...).
    """
    seen: dict[str, int] = {}
    for task in tasks:
        base = task.get("task_id") or "fix"
        count = seen.get(base, 0) + 1
        seen[base] = count
        if count > 1:
            task["task_id"] = f"{base}-{count}"
    return tasks


def plan_fix_tasks(design_plan: dict[str, Any], chief_decision: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    tasks=[]
    plan_tests = design_plan.get("tests", [])
    plan_criteria = design_plan.get("acceptance_criteria", [])
    plan_sources = design_plan.get("openspec_sources", [])
    for idx, step in enumerate(design_plan.get("edit_sequence", []), 1):
        files=step.get("files") or step.get("allowed_files") or chief_decision.get("fix_policy", {}).get("allowed_files") or []
        criteria = step.get("acceptance_criteria") or plan_criteria
        finding_ids = step.get("finding_ids") or ([step.get("finding_id")] if step.get("finding_id") else [])
        tasks.append(
            {
                "task_id": step.get("task_id") or step.get("id") or f"fix-{idx}",
                "summary": step.get("summary") or str(step),
                "allowed_files": files,
                "tests": step.get("tests") or plan_tests,
                "acceptance_criteria": criteria,
                "source_finding_ids": finding_ids,
                "openspec_sources": step.get("openspec_sources") or plan_sources,
                "openspec_backed": bool(step.get("openspec_backed") or design_plan.get("openspec_backed")),
            }
        )
    manifest={
        "schema_version":"fix-dispatch-task-manifest.v1",
        "tasks":_ensure_unique_task_ids(merge_tasks_touching_same_files(tasks)),
        "plan_hash":design_plan.get("plan_hash"),
        "fix_policy":chief_decision.get("fix_policy", config.get("autofix", {})),
        "openspec_backed": bool(design_plan.get("openspec_backed")),
        "openspec_sources": plan_sources,
    }
    # An approved design with nothing concrete to change is a no-op (LGTM), not a blocker.
    if not manifest["tasks"]:
        manifest["no_fix_needed"] = True
        return manifest
    validate_task_manifest(manifest)
    return manifest


def validate_task_manifest(manifest: dict[str, Any]) -> None:
    tasks=manifest.get("tasks", [])
    ids=[t.get("task_id") for t in tasks]
    if len(ids)!=len(set(ids)): raise ValidationError("duplicate task_id in manifest")
    for t in tasks:
        if not t.get("allowed_files"): raise ValidationError(f"task missing allowed_files: {t.get('task_id')}")
        if manifest.get("openspec_backed") and not t.get("acceptance_criteria"):
            raise ValidationError(f"OpenSpec-backed task missing acceptance_criteria: {t.get('task_id')}")


def write_fix_task_manifest(manifest: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, manifest, "fix-dispatch-task-manifest.v1")
