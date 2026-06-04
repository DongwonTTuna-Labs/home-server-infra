from codex_review.github.issues import make_issue_idempotency_key, build_deferred_issue_body


def test_issue_idempotency_key_is_deterministic():
    a=make_issue_idempotency_key("o/r", 1, "root")
    b=make_issue_idempotency_key("o/r", 1, "root")
    c=make_issue_idempotency_key("o/r", 2, "root")
    assert a == b
    assert a != c


def test_deferred_issue_body_contains_marker_and_source_thread():
    body=build_deferred_issue_body("root", [{"thread_id":"T1","path":"src/a.py","line":3}], {"repository":"o/r","pr_number":1})
    assert "codex-review:deferred-issue" in body
    assert "T1" in body
