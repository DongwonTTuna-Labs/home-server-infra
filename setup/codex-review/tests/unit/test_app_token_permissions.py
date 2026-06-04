import json
import pytest

from codex_review.github import app_token


def test_dispatch_mode_requests_minimal_repository_dispatch_permissions():
    assert app_token.permissions_for_write_mode("dispatch") == {
        "contents": "write",
        "pull_requests": "read",
    }


def _fake_rest_factory(repo_permissions=None, listing_permissions=None):
    repo_permissions = repo_permissions or {"pull": True}
    listing_permissions = listing_permissions or {}
    def fake_rest(method, url, token, body=None):
        if url.endswith("/installation/repositories"):
            return {"repositories": [{"full_name": "owner/repo"}], "permissions": listing_permissions}
        if "/installation/repositories?" in url:
            return {"repositories": [{"full_name": "owner/repo", "permissions": repo_permissions}]}
        if url.endswith("/repos/owner/repo") or url.endswith("/repos/owner/repo/"):
            return {"permissions": repo_permissions}
        return {}
    return fake_rest


def test_repo_scoped_token_requires_declared_granular_write_permissions(monkeypatch):
    monkeypatch.setattr(app_token, "rest_request", _fake_rest_factory(repo_permissions={"pull": True}))
    with pytest.raises(Exception, match="permission preflight"):
        app_token.assert_installation_token_for_repo("tok", "owner", "repo", {"issues": "write"})


def test_repo_scoped_token_accepts_threaded_creation_permissions(monkeypatch):
    monkeypatch.setattr(app_token, "rest_request", _fake_rest_factory(repo_permissions={"pull": True}))
    monkeypatch.setenv("CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON", json.dumps({"issues": "write", "pull_requests": "write", "contents": "read"}))
    result = app_token.assert_installation_token_for_repo("tok", "owner", "repo", {"issues": "write", "pull_requests": "write"})
    assert result["repository_scoped"] is True
    assert set(result["permission_preflight"]) >= {"issues", "pull_requests"}


def test_contents_write_can_be_proved_by_repo_push_permission(monkeypatch):
    monkeypatch.setattr(app_token, "rest_request", _fake_rest_factory(repo_permissions={"pull": True, "push": True}))
    result = app_token.assert_installation_token_for_repo("tok", "owner", "repo", {"contents": "write"})
    assert result["permission_preflight"]["contents"].startswith("permission_map")


def test_app_credentials_accept_current_codex_app_secret_names(monkeypatch):
    for name in (
        "CODEX_REVIEW_GITHUB_APP_ID",
        "CODEX_REVIEW_APP_ID",
        "GITHUB_APP_ID",
        "APP_ID",
        "CODEX_REVIEW_GITHUB_APP_PRIVATE_KEY",
        "CODEX_REVIEW_APP_PRIVATE_KEY",
        "GITHUB_APP_PRIVATE_KEY",
        "APP_PRIVATE_KEY",
    ):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("CODEX_APP_ID", "12345")
    monkeypatch.setenv("CODEX_APP_PRIVATE_KEY", "line1\\nline2")

    creds = app_token.load_app_credentials_from_env()

    assert creds == {"app_id": "12345", "private_key": "line1\nline2"}


def test_repo_scoped_token_checks_repo_root_without_trailing_slash(monkeypatch):
    requested_urls = []

    def fake_rest(method, url, token, body=None):
        requested_urls.append(url)
        if url.endswith("/installation/repositories"):
            return {"repositories": [{"full_name": "owner/repo"}], "permissions": {}}
        if "/installation/repositories?" in url:
            return {"repositories": [{"full_name": "owner/repo", "permissions": {"pull": True, "push": True}}]}
        if url.endswith("/repos/owner/repo"):
            return {"permissions": {"pull": True, "push": True}}
        raise AssertionError(f"unexpected GitHub API URL: {url}")

    monkeypatch.setattr(app_token, "rest_request", fake_rest)

    app_token.assert_installation_token_for_repo("tok", "owner", "repo", {"contents": "write"})

    assert "https://api.github.com/repos/owner/repo" in requested_urls
    assert "https://api.github.com/repos/owner/repo/" not in requested_urls
