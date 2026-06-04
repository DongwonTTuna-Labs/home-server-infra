"""Combine review axis findings."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import read_json, write_json
from codex_review.core.errors import ValidationError

_SEVERITY_RANK = {"critical": 0, "high": 1, "medium": 2, "low": 3, "info": 4}


def detect_cross_axis_duplicates(findings: list[dict[str, Any]]) -> list[dict[str, Any]]:
    groups={}
    for f in findings:
        groups.setdefault(f.get("root_cause_key") or f.get("finding_id"), []).append(f.get("finding_id") or f.get("id"))
    return [{"root_cause_key": k, "finding_ids": v} for k,v in groups.items() if len(v)>1]


def summarize_findings_by_axis(findings: list[dict[str, Any]]) -> dict[str, int]:
    summary={}
    for f in findings: summary[f.get("axis", "unknown")]=summary.get(f.get("axis", "unknown"),0)+1
    return summary


def cap_combined_findings(findings: list[dict[str, Any]], max_findings: int | None) -> tuple[list[dict[str, Any]], int]:
    """Bound the total findings handed to the techlead, keeping the most severe.

    Without a cap, 5 axes can each contribute up to ``max_findings_per_axis`` findings,
    so the techlead prompt grows unbounded. We keep the highest-severity findings
    (ties broken by original order) and report how many were dropped — never a silent
    truncation. Returns ``(findings, dropped_count)``.
    """
    if not max_findings or max_findings <= 0 or len(findings) <= max_findings:
        return findings, 0
    indexed = list(enumerate(findings))
    indexed.sort(key=lambda pair: (_SEVERITY_RANK.get(str(pair[1].get("severity") or "medium"), 2), pair[0]))
    kept = sorted(indexed[:max_findings], key=lambda pair: pair[0])
    return [f for _, f in kept], len(findings) - max_findings


def combine_axis_findings(axis_artifacts: list[dict[str, Any] | str | Path], config: dict[str, Any] | None = None) -> dict[str, Any]:
    all_findings=[]; ids=set()
    for art in axis_artifacts:
        payload=read_json(art) if isinstance(art, (str, Path)) else art
        axis=payload.get("axis")
        for f in payload.get("findings", []) or []:
            fid=f.get("finding_id") or f.get("id")
            if fid in ids: raise ValidationError(f"duplicate finding id across axes: {fid}")
            ids.add(fid)
            nf=dict(f); nf.setdefault("axis", axis); nf.setdefault("finding_id", fid)
            all_findings.append(nf)
    max_combined = (config or {}).get("review", {}).get("max_combined_findings")
    kept, dropped = cap_combined_findings(all_findings, max_combined)
    return {
        "schema_version": "review-combined-findings.v1",
        "findings": kept,
        "finding_count": len(kept),
        "total_finding_count": len(all_findings),
        "combined_truncated": dropped > 0,
        "dropped_finding_count": dropped,
        "duplicates": detect_cross_axis_duplicates(kept),
        "summary_by_axis": summarize_findings_by_axis(kept),
    }


def write_combined_findings(combined: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, combined, "review-combined-findings.v1")
