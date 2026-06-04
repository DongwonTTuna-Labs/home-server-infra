from codex_review.github import app_token


def test_create_installation_token_for_repo_scopes_repository(monkeypatch):
    calls = []
    monkeypatch.setattr(app_token, "load_app_credentials_from_env", lambda: {"app_id":"1", "private_key":"key"})
    monkeypatch.setattr(app_token, "create_jwt", lambda app_id, private_key: "jwt")
    monkeypatch.setattr(app_token, "get_installation_id", lambda owner, repo, jwt: 123)
    def fake_create(installation_id, permissions, jwt, **kwargs):
        calls.append(kwargs)
        return "tok"
    monkeypatch.setattr(app_token, "create_installation_token", fake_create)
    assert app_token.create_installation_token_for_repo("owner", "repo", {"contents":"write"}) == "tok"
    assert calls == [{"repositories": ["repo"]}]


def test_assert_installation_token_for_repo_rejects_other_repo(monkeypatch):
    def fake_rest(method, url, token, body=None):
        if url.endswith("/installation/repositories"):
            return {"repositories": []}
        if "/installation/repositories?" in url:
            return {"repositories": [{"full_name": "owner/other"}]}
        return {"permissions": {"push": True}}
    monkeypatch.setattr(app_token, "rest_request", fake_rest)
    try:
        app_token.assert_installation_token_for_repo("tok", "owner", "repo", {"contents":"write"})
    except Exception as exc:
        assert "not scoped" in str(exc)
    else:
        raise AssertionError("expected repository scope validation failure")
