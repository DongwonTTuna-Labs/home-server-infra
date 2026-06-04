"""Tests for token budgeting and context-size bounding (Workstream A)."""
from __future__ import annotations

import pytest

from codex_review.context.budget import compact_json
from codex_review.context.diff import hunk_headers
from codex_review.context.openspec import render_openspec_context_markdown, sections_for_stage
from codex_review.context.pr import build_pr_context, context_truncation_evidence
from codex_review.context.budget import estimate_tokens, fit_to_budget, within_budget
from codex_review.core.errors import ValidationError
from codex_review.stages.review.combine import cap_combined_findings, combine_axis_findings
from codex_review.stages.fix_merge.semantic_safety import build_semantic_patch_safety_prompt


def test_estimate_and_fit_to_budget():
    assert estimate_tokens("") == 0
    assert estimate_tokens("abcd") == 1
    assert estimate_tokens("a" * 4001) == 1001
    fitted, truncated = fit_to_budget("x" * 1000, 10)
    assert truncated is True
    assert "[truncated]" in fitted
    assert within_budget("short", 10)
    kept, untruncated = fit_to_budget("hello", 100)
    assert untruncated is False and kept == "hello"


def test_compact_json_is_deterministic_and_boundable():
    text = compact_json({"b": 1, "a": 2})
    assert text.index('"a"') < text.index('"b"')  # sorted keys
    bounded = compact_json({"k": "v" * 1000}, max_chars=50)
    assert bounded.endswith("...[truncated]") and len(bounded) <= 50 + len("\n...[truncated]")


def test_hunk_headers_keeps_only_at_lines():
    patch = "diff --git a/x b/x\n@@ -1,2 +1,3 @@\n+added\n-removed\n@@ -10 +11 @@\n context"
    assert hunk_headers(patch) == "@@ -1,2 +1,3 @@\n@@ -10 +11 @@"


def test_build_pr_context_budgets_large_patches_but_keeps_changed_line_map():
    big_patch = "@@ -1,1 +1,2000 @@\n" + "\n".join(f"+line {i}" for i in range(1, 2001))
    files = [
        {"filename": "src/big.rs", "status": "modified", "additions": 2000, "deletions": 0, "patch": big_patch},
        {"filename": "src/small.rs", "status": "modified", "additions": 1, "deletions": 0, "patch": "@@ -1 +1 @@\n+ok"},
    ]
    diff = "\n".join(f.get("patch") for f in files)
    ctx = build_pr_context({}, {"number": 1, "title": "t", "head": {}, "base": {}}, files, diff, {})

    big = next(f for f in ctx["changed_files"] if f["filename"] == "src/big.rs")
    assert big.get("patch_truncated") is True
    assert big["patch"].startswith("@@")  # reduced to hunk headers
    assert "line 1500" not in big["patch"]  # body dropped
    assert ctx["patches_truncated"] is True and ctx["truncated_patch_count"] == 1
    # changed-line map is derived from the FULL diff, so inline enforcement is intact.
    assert "src/big.rs" in ctx["changed_line_map"]
    assert len(ctx["changed_line_map"]["src/big.rs"]) == 2000
    assert context_truncation_evidence(ctx)["truncated_patch_count"] == 1


def test_build_pr_context_signals_diff_truncation():
    ctx = build_pr_context({}, {"number": 2, "head": {}, "base": {}}, [], "x" * 20000, {"context": {"diff_summary_tokens": 100}})
    assert ctx["diff_truncated"] is True
    assert context_truncation_evidence(ctx) is not None


def test_no_truncation_evidence_when_small():
    ctx = build_pr_context({}, {"number": 3, "head": {}, "base": {}}, [{"filename": "a", "patch": "@@ -1 +1 @@\n+x"}], "@@ -1 +1 @@\n+x", {})
    assert ctx["diff_truncated"] is False and ctx["patches_truncated"] is False
    assert context_truncation_evidence(ctx) is None


def _finding(fid, severity):
    return {"schema_version": "shared-review-finding.v1", "finding_id": fid, "severity": severity, "file": "a.rs", "line": 1, "root_cause_key": fid}


def test_cap_combined_findings_keeps_highest_severity():
    findings = [_finding("low1", "low"), _finding("crit1", "critical"), _finding("med1", "medium")]
    kept, dropped = cap_combined_findings(findings, 2)
    ids = {f["finding_id"] for f in kept}
    assert ids == {"crit1", "med1"} and dropped == 1


def test_combine_axis_findings_caps_total_with_config():
    axis_a = {"axis": "security", "findings": [_finding("a1", "critical"), _finding("a2", "low")]}
    axis_b = {"axis": "correctness", "findings": [_finding("b1", "high"), _finding("b2", "info")]}
    combined = combine_axis_findings([axis_a, axis_b], {"review": {"max_combined_findings": 2}})
    assert combined["combined_truncated"] is True
    assert combined["dropped_finding_count"] == 2
    assert combined["finding_count"] == 2
    assert combined["total_finding_count"] == 4
    assert {f["finding_id"] for f in combined["findings"]} == {"a1", "b1"}


def test_combine_axis_findings_no_cap_when_unset():
    axis_a = {"axis": "security", "findings": [_finding("a1", "low"), _finding("a2", "low")]}
    combined = combine_axis_findings([axis_a])
    assert combined["combined_truncated"] is False and combined["dropped_finding_count"] == 0


def test_sections_for_stage_routing():
    assert sections_for_stage("review_security") == {"proposal", "spec", "other"}
    assert "tasks" in sections_for_stage("design_plan")
    assert sections_for_stage(None) is None


def _openspec_ctx():
    return {
        "schema_version": "openspec-context.v1",
        "present": True,
        "status": "ready",
        "source_summary": ["owner/repo:openspec/changes/x/tasks.md"],
        "documents": [
            {"path": "openspec/changes/x/proposal.md", "content": "PROPOSAL BODY"},
            {"path": "openspec/changes/x/tasks.md", "content": "TASKS BODY"},
        ],
    }


def test_render_openspec_filters_by_section():
    md = render_openspec_context_markdown(_openspec_ctx(), sections={"tasks", "other"})
    assert "TASKS BODY" in md and "PROPOSAL BODY" not in md


def test_render_openspec_budget_truncates():
    ctx = {"present": True, "status": "ready", "source_summary": [], "documents": [{"path": "openspec/changes/x/tasks.md", "content": "T" * 8000}]}
    md = render_openspec_context_markdown(ctx, budget_tokens=50)
    assert "truncated to fit token budget" in md


def test_semantic_safety_prompt_fails_closed_when_over_budget():
    merged_fix = {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": "diff --git a/x b/x\n" + ("+l\n" * 4000), "expected_head_sha": "abc"}
    with pytest.raises(ValidationError, match="too large for single-pass"):
        build_semantic_patch_safety_prompt(merged_fix, {"title": "t", "head_sha": "abc"}, "", token_budget=100)


def test_semantic_safety_prompt_ok_within_budget():
    merged_fix = {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": "diff --git a/x b/x\n@@ -1 +1 @@\n+ok", "expected_head_sha": "abc"}
    prompt = build_semantic_patch_safety_prompt(merged_fix, {"title": "t", "head_sha": "abc"}, "", token_budget=100000)
    assert "Semantic Patch Safety Review" in prompt
