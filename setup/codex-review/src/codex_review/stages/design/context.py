"""Stage03 design context."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json


def select_design_relevant_files(techlead_decision: dict[str, Any], pr_context: dict[str, Any]) -> list[str]:
    files=set()
    for item in techlead_decision.get("decisions", []) or techlead_decision.get("all_decisions", []):
        if item.get("file"): files.add(item["file"])
        for f in item.get("files", []) or []: files.add(f)
    for f in pr_context.get("changed_files_summary", []) or []:
        if f.get("filename"): files.add(f["filename"])
    return sorted(files)


def _design_findings(source_decision: dict[str, Any]) -> list[dict[str, Any]]:
    findings=[]
    for item in source_decision.get("decisions", []) or source_decision.get("all_decisions", []):
        action = item.get("action")
        state = item.get("state")
        if action in {"needs_design", "publish_and_fix_now", "summary_only_fix_now"} or state == "fix_now":
            finding = dict(item)
            finding.setdefault("action", "needs_design")
            finding.setdefault("finding_id", item.get("finding_id") or item.get("thread_id"))
            finding.setdefault("summary", item.get("summary") or item.get("reason") or item.get("evidence") or "Existing unresolved review thread needs design")
            findings.append(finding)
    return findings


def include_current_source_windows(context: dict[str, Any], files: list[dict[str, Any]] | list[str]) -> dict[str, Any]:
    context["source_windows"] = files
    return context


def build_design_context(
    pr_context: dict[str, Any],
    techlead_decision: dict[str, Any],
    review_context: str,
    docs_context: str,
    file_inventory: dict[str, Any] | list[str] | None = None,
    openspec_context: dict[str, Any] | None = None,
) -> dict[str, Any]:
    relevant = file_inventory if file_inventory is not None else select_design_relevant_files(techlead_decision, pr_context)
    open_ctx = openspec_context or {}
    return {
        "schema_version":"design-context.v1",
        "pr_context":pr_context,
        "techlead_decision":techlead_decision,
        "review_context":review_context,
        "docs_context":docs_context,
        "openspec_context":open_ctx,
        "openspec_backed": bool(open_ctx.get("present")),
        "relevant_files":relevant,
        "findings":_design_findings(techlead_decision),
    }


def write_design_context(context: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, context, "design-context.v1")
