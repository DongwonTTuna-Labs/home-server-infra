"""Stage01 axis finding validation."""
from __future__ import annotations
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.constants import SEVERITIES
from codex_review.context.diff import is_changed_right_line
from codex_review.core.errors import ValidationError
from codex_review.model.inspection import validate_inspection_evidence
from codex_review.core.schema import validate_enum
from codex_review.security.redaction import assert_no_secret_patterns
from .axes import validate_axis

AXIS_ALIASES = {
    "project-specific-correctness": "domain",
    "project-specific correctness and product requirements": "domain",
}


def validate_finding_id(axis: str, finding_id: str) -> None:
    if not finding_id or not str(finding_id).strip():
        raise ValidationError("finding id must be non-empty")


def validate_finding_location(finding: dict[str, Any], changed_line_map: dict[str, Any]) -> None:
    file=finding.get("file") or finding.get("path")
    line=finding.get("line")
    if file is None or line is None:
        raise ValidationError("finding must include file and line")
    if changed_line_map and not is_changed_right_line(changed_line_map, str(file), int(line)):
        raise ValidationError(f"finding location is not a changed RIGHT-side line: {file}:{line}")


def validate_root_cause_key(finding: dict[str, Any]) -> None:
    if not finding.get("root_cause_key"):
        raise ValidationError("finding missing root_cause_key")


def redact_and_validate_finding_text(finding: dict[str, Any]) -> dict[str, Any]:
    for key in ["title", "summary", "details", "recommendation", "body"]:
        if finding.get(key):
            assert_no_secret_patterns(str(finding[key]), f"finding.{key}")
    return finding


def validate_axis_findings(
    axis: str,
    findings: dict[str, Any] | list[dict[str, Any]],
    pr_context: dict[str, Any] | None,
    changed_line_map: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path | None = None,
) -> dict[str, Any]:
    validate_axis(axis, config)
    payload = {"findings": findings} if isinstance(findings, list) else dict(findings)
    evidence = validate_inspection_evidence(payload, repo_path, f"review {axis}")
    payload_axis = AXIS_ALIASES.get(str(payload.get("axis") or ""), payload.get("axis"))
    if payload_axis and payload_axis != axis:
        raise ValidationError(f"axis mismatch: {payload.get('axis')} != {axis}")
    items=payload.get("findings") or []
    ids=set()
    require_changed=config.get("review", {}).get("require_changed_right_line", True)
    for f in items:
        fid=f.get("finding_id") or f.get("id")
        if fid in ids: raise ValidationError(f"duplicate finding id: {fid}")
        ids.add(fid)
        validate_finding_id(axis, str(fid))
        validate_enum(f.get("severity", "medium"), SEVERITIES, "severity")
        if require_changed: validate_finding_location(f, changed_line_map)
        validate_root_cause_key(f)
        redact_and_validate_finding_text(f)
    return {
        "schema_version": "review-axis-findings.v1",
        "axis": axis,
        "findings": items,
        "finding_count": len(items),
        "inspection_evidence": evidence,
    }


def write_validated_axis_findings(axis: str, findings: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, findings, "review-axis-findings.v1")
