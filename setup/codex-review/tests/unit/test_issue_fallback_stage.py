from __future__ import annotations

import pytest

from codex_review.stages.issue_fallback.issue import (
    apply_issue_fallback,
    build_issue_fallback_plan,
    compose_issue_content,
    infer_issue_reason,
    render_issue_fallback_body,
)


def test_issue_fallback_plan_is_idempotent_and_openspec_aware():
    pr_context = {
        "owner": "DongwonTTuna-Labs",
        "repo": "rs-builder-relayer-client",
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": 41,
        "html_url": "https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/pull/41",
    }
    openspec_context = {
        "present": True,
        "source_summary": ["openspec/changes/demo/tasks.md"],
    }

    plan = build_issue_fallback_plan(
        reason="missing_openspec_spec",
        pr_context=pr_context,
        openspec_context=openspec_context,
        attempted_stages=["design", "design_chief"],
    )

    assert plan["schema_version"] == "issue-fallback-issue-fallback.v1"
    assert plan["idempotency_key"]
    assert "missing_openspec_spec" in plan["title"]
    assert "openspec/changes/demo/tasks.md" in plan["body"]
    assert "codex-review:issue-fallback" in plan["body"]


def test_issue_fallback_body_names_required_follow_up():
    body = render_issue_fallback_body(
        idempotency_key="abc123",
        reason="no-diff-repeat",
        pr_context={"pr_number": 41},
        openspec_context={"present": False, "decision": "missing_openspec_spec"},
        attempted_stages=["push"],
        required_follow_up="Create an OpenSpec change or adjust the PR body link.",
    )

    assert "필요한 후속조치" in body
    assert "Create an OpenSpec change" in body
    assert "원본 PR: #41" in body


def test_issue_fallback_plan_includes_techlead_deferred_items():
    plan = build_issue_fallback_plan(
        reason="techlead_defer_to_issue",
        pr_context={"owner":"o", "repo":"r", "repository":"o/r", "pr_number":7},
        openspec_context={"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
        deferred_items=[{
            "finding_id": "F-1",
            "root_cause_key": "outside-pr-scope",
            "title": "Move unrelated migration to a follow-up",
            "file": "src/lib.rs",
            "line": 12,
            "recommendation": "Track this outside the current PR branch.",
        }],
    )

    assert plan["deferred_count"] == 1
    assert "이관된 지적사항" in plan["body"]
    assert "F-1" in plan["body"]
    assert "outside-pr-scope" in plan["body"]
    assert "techlead_defer_to_issue" in plan["title"]


def test_issue_fallback_actual_apply_requires_app_token():
    plan = {
        "idempotency_key": "abc",
        "title": "Codex review fallback: demo",
        "body": "body",
    }
    with pytest.raises(Exception, match="GitHub App installation token"):
        apply_issue_fallback(plan, {"owner": "o", "repo": "r"}, None, dry_run=False)


def test_issue_fallback_dry_run_remains_explicit():
    plan = {
        "idempotency_key": "abc",
        "title": "Codex review fallback: demo",
        "body": "body",
    }
    result = apply_issue_fallback(plan, {"owner": "o", "repo": "r"}, None, dry_run=True)
    assert result["status"] == "dry_run"


def test_issue_fallback_no_diff_repeat_uses_specific_follow_up():
    plan = build_issue_fallback_plan(
        reason="no_diff_repeat",
        pr_context={"owner":"o", "repo":"r", "repository":"o/r", "pr_number":7},
        openspec_context={"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
        attempted_stages=["push"],
    )

    assert "비어있지 않은 패치" in plan["required_follow_up"]
    assert "no_diff_repeat" in plan["title"]


def test_infer_reason_prioritizes_terminal_fix_loop():
    inferred = infer_issue_reason(
        fix_validation={"status": "blocked", "loop_terminal_reason": "oscillation_detected"},
        design_route={"route": "run_fix_dispatch"},
        review_publication={"deferred_items": [{"finding_id": "F-1"}]},
    )
    assert inferred["reason"] == "oscillation_detected"
    assert "push" in inferred["attempted_stages"]


def test_infer_reason_falls_back_to_deferred_items():
    inferred = infer_issue_reason(
        review_publication={"deferred_items": [{"finding_id": "F-2", "title": "x"}]},
    )
    assert inferred["reason"] == "techlead_defer_to_issue"
    assert inferred["deferred_items"] and inferred["deferred_items"][0]["finding_id"] == "F-2"


def test_infer_reason_reports_missing_artifacts():
    inferred = infer_issue_reason()
    assert inferred["reason"] == "artifacts_missing"


def test_compose_keeps_deterministic_body_when_model_drops_marker():
    plan = build_issue_fallback_plan(
        reason="manual_fallback",
        pr_context={"owner": "o", "repo": "r", "repository": "o/r", "pr_number": 9},
        openspec_context={"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
    )
    composed = compose_issue_content(plan, {"title": "정리된 제목", "body": "마커 없는 본문"})
    assert composed["title"] == "정리된 제목"
    # Marker missing -> deterministic body retained for idempotency.
    assert "codex-review:issue-fallback" in composed["body"]
    assert composed["body_polish_skipped"] == "missing_marker"


def test_compose_accepts_model_body_with_marker():
    plan = build_issue_fallback_plan(
        reason="manual_fallback",
        pr_context={"owner": "o", "repo": "r", "repository": "o/r", "pr_number": 9},
        openspec_context={"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
    )
    polished_body = plan["body"] + "\n\n사람이 읽기 좋은 추가 설명."
    composed = compose_issue_content(plan, {"title": "정리된 제목", "body": polished_body})
    assert composed["body"].endswith("추가 설명.")
    assert "body_polish_skipped" not in composed
