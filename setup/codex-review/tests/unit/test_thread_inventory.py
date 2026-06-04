import pytest
from codex_review.stages.resolve_gate.collect import collect_thread_inventory


def cfg():
    return {"trusted": {"codex_review_authors": ["codex-bot"]}}


def thread(tid, commit="old", body='<!-- codex-review:inline {"finding_id":"f1","root_cause_key":"rc1"} -->', resolved=False):
    return {"id": tid, "isResolved": resolved, "path":"src/a.py", "line": 5, "comments":[{"id":"c"+tid, "body":body, "author":{"login":"codex-bot"}, "path":"src/a.py", "line":5, "commit":{"oid":commit}}]}


def test_collects_previous_head_trusted_codex_threads_only():
    inv=collect_thread_inventory({"head_sha":"new","pr_number":1}, [thread("T1")], [], cfg())
    assert inv["thread_count"] == 1
    assert inv["items"][0]["root_cause_key"] == "rc1"


def test_excludes_resolved_and_current_head_threads():
    inv=collect_thread_inventory({"head_sha":"new"}, [thread("old"), thread("new", commit="new"), thread("done", resolved=True)], [], cfg())
    assert [i["thread_id"] for i in inv["items"]] == ["old"]


def test_invalid_marker_forces_needs_human():
    inv=collect_thread_inventory({"head_sha":"new"}, [thread("T1", body="<!-- codex-review:inline -->")], [], cfg())
    assert inv["items"][0]["forced_needs_human"] is True
