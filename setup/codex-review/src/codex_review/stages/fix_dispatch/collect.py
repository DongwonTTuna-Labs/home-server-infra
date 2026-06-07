"""Collect fix agent results."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import read_json, write_json
from codex_review.core.errors import ValidationError


def classify_agent_result_statuses(results: list[dict[str, Any]]) -> dict[str, int]:
    counts={}
    for r in results: counts[r.get("status", "patched")]=counts.get(r.get("status", "patched"),0)+1
    return counts


def build_fix_collection_result(manifest: dict[str, Any], results: list[dict[str, Any]], missing_task_ids: list[str] | None = None) -> dict[str, Any]:
    task_ids=[t.get("task_id") for t in manifest.get("tasks", [])]
    result_ids=[r.get("task_id") for r in results]
    missing=missing_task_ids if missing_task_ids is not None else [tid for tid in task_ids if tid not in result_ids]
    return {"schema_version":"fix-dispatch-collection-result.v1","manifest_task_count":len(task_ids),"results":results,"status_counts":classify_agent_result_statuses(results),"missing_task_ids":missing,"ready_for_merge": all(r.get("status", "patched")=="patched" for r in results) and bool(results) and not missing}


def collect_agent_results(manifest: dict[str, Any], result_paths: list[str | Path]) -> dict[str, Any]:
    tasks={t.get("task_id"): t for t in manifest.get("tasks", [])}
    selected: dict[str, tuple[dict[str, Any], int, Path]] = {}
    for p in result_paths:
        artifact_path = Path(p)
        priority = 1 if artifact_path.name.endswith(".validated.json") else 0
        artifact_parent = artifact_path.parent.resolve()
        payload=read_json(p)
        if payload.get("schema_version") != "fix-dispatch-agent-result.v1":
            continue
        tid=payload.get("task_id")
        if tid not in tasks:
            # The fix agent sometimes emits an extra result dir keyed by a
            # finding id rather than a manifest task id. The manifest is the
            # source of truth, so ignore results for unknown tasks instead of
            # failing the whole loop; manifest tasks with no result are still
            # reported as missing (no_safe_fix) below.
            continue
        if tid in selected and selected[tid][2] != artifact_parent:
            raise ValidationError(f"duplicate fix result for task_id: {tid}")
        if tid in selected and selected[tid][1] == priority:
            raise ValidationError(f"duplicate fix result for task_id: {tid}")
        if tid not in selected or priority > selected[tid][1]:
            selected[tid] = (payload, priority, artifact_parent)
    results=[]
    missing=[]
    for tid in tasks:
        if tid in selected:
            results.append(selected[tid][0])
        else:
            missing.append(tid)
            results.append({"schema_version":"fix-dispatch-agent-result.v1","task_id":tid,"status":"no_safe_fix","reason":"no agent result artifact was provided","defaulted":True})
    return build_fix_collection_result(manifest, results, missing)


def write_fix_collection_result(collection: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, collection, "fix-dispatch-collection-result.v1")
