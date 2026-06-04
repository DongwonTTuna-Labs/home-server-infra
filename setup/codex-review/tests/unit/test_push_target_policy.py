import pytest

from codex_review.stages.push.validate import validate_push_target


def test_push_target_rejects_fork_pr():
    with pytest.raises(Exception):
        validate_push_target({"same_repo": False, "head_repo_full_name":"fork/repo", "base_repo_full_name":"base/repo", "head_ref":"branch", "owner":"base", "repo":"repo", "pr_number":1})


def test_push_target_accepts_same_repo_pr():
    validate_push_target({"same_repo": True, "head_repo_full_name":"base/repo", "base_repo_full_name":"base/repo", "head_ref":"branch", "owner":"base", "repo":"repo", "pr_number":1})
