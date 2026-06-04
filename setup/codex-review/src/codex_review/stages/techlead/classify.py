"""Build publication artifact from techlead decisions."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json


def determine_techlead_status(decision: dict[str, Any]) -> str:
    if decision.get("status"): return decision["status"]
    items=decision.get("decisions", [])
    if any(i.get("action")=="needs_human" for i in items): return "needs_human"
    if any(i.get("action") in {"publish_and_fix_now","summary_only_fix_now","needs_design"} for i in items): return "needs_design"
    return "publishable" if items else "lgtm"


def determine_needs_design(decision: dict[str, Any]) -> bool:
    return bool(decision.get("needs_design") or determine_techlead_status(decision)=="needs_design")


def build_review_publication(decision: dict[str, Any], combined_findings: dict[str, Any], config: dict[str, Any]) -> dict[str, Any]:
    by_id={f.get("finding_id") or f.get("id"): f for f in combined_findings.get("findings", [])}
    inline=[]; summary=[]; deferred=[]
    for item in decision.get("decisions", []):
        finding=by_id.get(item.get("finding_id"), {})
        merged={**finding, **item}
        action=item.get("action")
        if action in {"publish_and_fix_now", "publish_only", "needs_design"}:
            inline.append(merged)
        elif action == "defer_to_issue":
            deferred.append(merged)
        elif action != "drop_duplicate":
            summary.append(merged)
    return {"schema_version":"techlead-review-publication.v1","status":determine_techlead_status(decision),"needs_design":determine_needs_design(decision),"inline_comments":inline,"summary_items":summary,"deferred_items":deferred,"all_decisions":decision.get("decisions", [])}


def write_review_publication(publication: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, publication, "techlead-review-publication.v1")
