import json
from codex_review.core.env import read_event_payload, resolve_repository_parts


def test_read_event_payload(tmp_path):
    p=tmp_path/"event.json"
    p.write_text(json.dumps({"number":1}), encoding="utf-8")
    assert read_event_payload(p)["number"] == 1


def test_resolve_repository_parts():
    assert resolve_repository_parts("owner/repo") == ("owner", "repo")


def test_workflow_dispatch_pr_number_from_inputs(tmp_path, monkeypatch):
    from codex_review.cli import _handle_event
    import argparse, json
    event = {
        "inputs": {"pr_number": "42"},
        "repository": {"full_name": "octo/repo", "name": "repo", "owner": {"login": "octo"}},
        "sender": {"login": "u"},
    }
    event_path = tmp_path / "event.json"
    event_path.write_text(json.dumps(event), encoding="utf-8")
    monkeypatch.setenv("GITHUB_EVENT_NAME", "workflow_dispatch")
    args = argparse.Namespace(command="resolve-current", in_path=str(event_path), event=None, out=None)
    out, schema = _handle_event(args)
    assert schema == "event-context.v1"
    assert out["pr_number"] == "42"
    assert out["repository"] == "octo/repo"
    assert out["same_repo"] is None
