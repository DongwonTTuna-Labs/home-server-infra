from codex_review.stages.techlead import publish as review_publish
from codex_review.stages.design_chief import publish as design_publish


def test_techlead_publish_dry_run_does_not_call_github(monkeypatch):
    called = []
    monkeypatch.setattr(review_publish, "create_pull_request_review", lambda *a, **k: called.append("review"))
    monkeypatch.setattr(review_publish, "upsert_sticky_comment", lambda *a, **k: called.append("summary"))
    publication = {
        "inline_comments": [{"file": "src/a.py", "line": 2, "summary": "x", "finding_id": "f1"}],
        "summary_items": [],
        "deferred_items": [],
    }
    report = review_publish.publish_review(publication, {"owner":"o","repo":"r","pr_number":1}, {"src/a.py":[2]}, "token", {"review":{}}, dry_run=True)
    assert report["dry_run"] is True
    assert called == []


def test_design_chief_publish_dry_run_does_not_call_github(monkeypatch):
    called = []
    monkeypatch.setattr(design_publish, "upsert_sticky_comment", lambda *a, **k: called.append("summary"))
    report = design_publish.publish_design_summary({"edit_sequence": [], "tests": []}, {"status":"no_fix_needed"}, "token", {"owner":"o","repo":"r","pr_number":1}, dry_run=True)
    assert report["dry_run"] is True
    assert called == []
