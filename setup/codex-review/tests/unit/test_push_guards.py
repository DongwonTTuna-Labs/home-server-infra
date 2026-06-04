import pytest
from codex_review.stages.push.validate import validate_current_head, validate_ready_to_push
from codex_review.stages.push.commit import build_commit_message


def test_head_sha_drift_blocks_push():
    with pytest.raises(Exception):
        validate_current_head({"head_sha":"abc","current_head_sha":"def"}, {"expected_head_sha":"abc","patch":"x"}, None)


def test_ready_to_push_requires_patch():
    with pytest.raises(Exception):
        validate_ready_to_push({})


def test_commit_message_contains_autofix_marker():
    msg=build_commit_message({"commit_plan":[{"subject":"docs(openspec): add lgtm loop guide","body":"Add the OpenSpec smoke guide.","paths":["docs/CODEX_REVIEW_LGTM_LOOP.md"]}]}, "plan", "old")
    assert "codex-review:autofix" in msg
    assert msg.startswith("docs(openspec): add lgtm loop guide\n\n")
    assert "Codex Review Autofix" not in msg


def test_commit_message_rejects_generic_autofix_subject():
    with pytest.raises(Exception):
        build_commit_message({"commit_plan":[{"subject":"Codex Review Autofix","paths":["docs/a.md"]}]}, "plan", "old")
