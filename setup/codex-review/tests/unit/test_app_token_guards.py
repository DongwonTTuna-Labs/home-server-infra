import pytest

from codex_review.github import app_token
from codex_review.stages.techlead import publish as review_publish
from codex_review.stages.design_chief import publish as design_publish
from codex_review.stages.push.orchestrate import commit_and_push_validated_fix


def test_assert_installation_token_rejects_non_installation_response(monkeypatch):
    monkeypatch.setattr(app_token, "rest_request", lambda *a, **k: {"message": "not installation"})
    with pytest.raises(Exception):
        app_token.assert_installation_token("tok", {"contents": "write"})


def test_techlead_actual_publish_requires_app_token(monkeypatch):
    monkeypatch.setattr(review_publish, "assert_installation_token_for_repo", lambda *a, **k: (_ for _ in ()).throw(Exception("not app")))
    pub = {"inline_comments": [], "summary_items": []}
    with pytest.raises(Exception):
        review_publish.publish_review(pub, {"owner":"o","repo":"r","pr_number":1,"head_sha":"h"}, {}, "tok", {}, dry_run=False)


def test_design_chief_actual_publish_requires_app_token(monkeypatch):
    monkeypatch.setattr(design_publish, "assert_installation_token_for_repo", lambda *a, **k: (_ for _ in ()).throw(Exception("not app")))
    with pytest.raises(Exception):
        design_publish.publish_design_summary({"edit_sequence":[],"tests":[]}, {"status":"no_fix_needed"}, "tok", {"owner":"o","repo":"r","pr_number":1}, dry_run=False)


def test_push_actual_push_without_token_fails(tmp_path):
    merged = {"schema_version":"fix-merge-merged-fix.v1", "status":"ready", "patch":"diff --git a/a b/a\nnew file mode 100644\nindex 0000000..7898192\n--- /dev/null\n+++ b/a\n@@ -0,0 +1 @@\n+x\n", "expected_head_sha":"h"}
    validation = {"schema_version":"push-validated-fix.v1", "status":"validated", "validated": True, "patch_hash": None, "semantic_safety_approved": True, "semantic_safety": {"status": "approved", "approved": True}}
    pr = {"head_sha":"h", "same_repo": True, "head_ref":"branch", "owner":"o", "repo":"r", "pr_number":1}
    with pytest.raises(Exception, match="GitHub App installation token"):
        commit_and_push_validated_fix(merged, validation, pr, {}, tmp_path, None, dry_run=False)
