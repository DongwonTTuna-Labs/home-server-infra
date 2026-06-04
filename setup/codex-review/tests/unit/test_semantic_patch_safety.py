import hashlib

import pytest

from codex_review.core.errors import ValidationError
from codex_review.stages.fix_merge.semantic_safety import (
    build_semantic_patch_safety_prompt,
    validate_semantic_patch_safety_result,
)


def test_semantic_safety_prompt_contains_exact_patch_hash_and_patch():
    patch = "diff --git a/src/a.txt b/src/a.txt\n--- a/src/a.txt\n+++ b/src/a.txt\n"
    prompt = build_semantic_patch_safety_prompt(
        {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": patch},
        {"title": "Implement OpenSpec change", "body": "Spec link"},
        "# OpenSpec context",
    )
    assert hashlib.sha256(patch.encode("utf-8")).hexdigest() in prompt
    assert patch in prompt
    assert "not a keyword blocker" in prompt
    assert "commit_plan" in prompt


def test_semantic_safety_validation_requires_exact_patch_hash():
    patch = "diff --git a/src/a.txt b/src/a.txt\n--- a/src/a.txt\n+++ b/src/a.txt\n"
    with pytest.raises(ValidationError):
        validate_semantic_patch_safety_result(
            {
                "schema_version": "fix-merge-semantic-patch-safety.v1",
                "status": "approved",
                "approved": True,
                "patch_hash": "wrong",
                "summary": "ok",
                "blocking_reason": None,
                "reviewed_criteria": [],
                "semantic_findings": [],
            },
            {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": patch},
        )


def test_semantic_safety_validation_allows_rejection_for_issue_fallback():
    patch = "diff --git a/src/a.txt b/src/a.txt\n--- a/src/a.txt\n+++ b/src/a.txt\n"
    result = validate_semantic_patch_safety_result(
        {
            "schema_version": "fix-merge-semantic-patch-safety.v1",
            "status": "needs_issue",
            "approved": False,
            "patch_hash": hashlib.sha256(patch.encode("utf-8")).hexdigest(),
            "summary": "Patch needs follow-up issue.",
            "blocking_reason": "Requires out-of-repo change.",
            "reviewed_criteria": ["scope"],
            "semantic_findings": [],
        },
        {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": patch},
    )
    assert result["status"] == "needs_issue"
    assert result["approved"] is False


def test_semantic_safety_validation_requires_meaningful_commit_plan_for_approved_patch():
    patch = "diff --git a/docs/a.md b/docs/a.md\n--- a/docs/a.md\n+++ b/docs/a.md\n"
    with pytest.raises(ValidationError):
        validate_semantic_patch_safety_result(
            {
                "schema_version": "fix-merge-semantic-patch-safety.v1",
                "status": "approved",
                "approved": True,
                "patch_hash": hashlib.sha256(patch.encode("utf-8")).hexdigest(),
                "summary": "ok",
                "blocking_reason": None,
                "reviewed_criteria": [],
                "semantic_findings": [],
                "commit_plan": [{"subject": "Codex Review Autofix", "body": "", "paths": ["docs/a.md"]}],
            },
            {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": patch},
        )


def test_semantic_safety_validation_accepts_commit_plan_for_approved_patch():
    patch = "diff --git a/docs/a.md b/docs/a.md\n--- a/docs/a.md\n+++ b/docs/a.md\n"
    result = validate_semantic_patch_safety_result(
        {
            "schema_version": "fix-merge-semantic-patch-safety.v1",
            "status": "approved",
            "approved": True,
            "patch_hash": hashlib.sha256(patch.encode("utf-8")).hexdigest(),
            "summary": "ok",
            "blocking_reason": None,
            "reviewed_criteria": [],
            "semantic_findings": [],
            "commit_plan": [{"subject": "docs(openspec): add lgtm loop smoke guide", "body": "Add the docs-only smoke guide.", "paths": ["docs/a.md"]}],
        },
        {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": patch},
    )
    assert result["commit_plan"][0]["subject"] == "docs(openspec): add lgtm loop smoke guide"
