from __future__ import annotations

import hashlib

import pytest

from codex_review.core.errors import PolicyViolation, ValidationError
from codex_review.loop.state import build_push_entry, fingerprint_patch
from codex_review.security.patch_policy import validate_patch_policy
from codex_review.stages.fix_merge.semantic_safety import build_semantic_patch_safety_prompt
from codex_review.stages.review.validate import validate_finding_location

CODE_PATCH = """diff --git a/src/app.py b/src/app.py
--- a/src/app.py
+++ b/src/app.py
@@ -1 +1 @@
-old
+new
"""

MEMORY_PATCH = """diff --git a/.omo/review-memory/pr-7/ledger.json b/.omo/review-memory/pr-7/ledger.json
--- a/.omo/review-memory/pr-7/ledger.json
+++ b/.omo/review-memory/pr-7/ledger.json
@@ -1 +1 @@
-{}
+{\"entries\": []}
"""

PLAN = {"edit_sequence": [{"task_id": "Task-1", "finding_ids": ["F1"]}]}


def test_memory_only_diff_contributes_no_oscillation_or_progress_metadata() -> None:
    fingerprint = fingerprint_patch(MEMORY_PATCH)
    entry = build_push_entry(1, {"commit_sha": "c1", "head_sha": "h1"}, MEMORY_PATCH, PLAN)

    assert fingerprint == {"added_line_fp": [], "removed_line_fp": [], "touched_paths": []}
    assert entry["patch_sha256"] == ""
    assert entry["normalized_finding_keys"] == []
    assert entry["added_line_fp"] == []
    assert entry["removed_line_fp"] == []
    assert entry["touched_paths"] == []


def test_code_plus_memory_diff_hashes_and_fingerprints_only_code_paths() -> None:
    code_entry = build_push_entry(1, {"commit_sha": "c1", "head_sha": "h1"}, CODE_PATCH, PLAN)
    mixed_entry = build_push_entry(1, {"commit_sha": "c1", "head_sha": "h1"}, CODE_PATCH + MEMORY_PATCH, PLAN)

    assert mixed_entry["patch_sha256"] == hashlib.sha256(CODE_PATCH.encode("utf-8")).hexdigest()
    assert mixed_entry["patch_sha256"] == code_entry["patch_sha256"]
    assert mixed_entry["normalized_finding_keys"] == code_entry["normalized_finding_keys"]
    assert mixed_entry["added_line_fp"] == code_entry["added_line_fp"]
    assert mixed_entry["removed_line_fp"] == code_entry["removed_line_fp"]
    assert mixed_entry["touched_paths"] == ["src/app.py"]


@pytest.mark.parametrize(
    "policy",
    [
        {"memory_write_prefix": ".omo/review-memory/"},
        {"allowed_prefixes": ["src/"], "memory_write_prefix": ".omo/review-memory/"},
        {"allowed_prefixes": [".omo/review-memory/"], "memory_write_prefix": ".omo/review-memory/"},
    ],
)
def test_patch_policy_rejects_review_memory_as_fix_target(policy: dict[str, object]) -> None:
    with pytest.raises(PolicyViolation, match="review memory path"):
        validate_patch_policy(MEMORY_PATCH, policy, {})


def test_semantic_safety_changed_file_context_excludes_memory_paths() -> None:
    prompt = build_semantic_patch_safety_prompt(
        {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": CODE_PATCH},
        {
            "title": "Implement code change",
            "body": "Body",
            "changed_files": [
                {"filename": "src/app.py", "patch": "@@ -1 +1 @@\n-old\n+new\n"},
                {"filename": ".omo/review-memory/pr-7/ledger.json", "patch": "@@ -1 +1 @@\n-{}\n+[]\n"},
                ".omo/review-memory/pr-7/learnings.md",
                {"new_path": ".omo/review-memory/pr-7/nested/scratch.txt", "old_path": "/dev/null"},
                {"path": "docs/readme.md"},
            ],
        },
        "# docs",
    )

    assert "src/app.py" in prompt
    assert "docs/readme.md" in prompt
    assert ".omo/review-memory" not in prompt


def test_review_finding_validation_rejects_memory_changed_line_targets() -> None:
    with pytest.raises(ValidationError, match="review memory path"):
        validate_finding_location(
            {"file": ".omo/review-memory/pr-7/ledger.json", "line": 1},
            {".omo/review-memory/pr-7/ledger.json": {1}},
        )
