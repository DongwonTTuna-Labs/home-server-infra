import json

import pytest

from codex_review.cli import main
from codex_review.core.errors import ValidationError
from codex_review.github import oidc_token


def _set_oidc_env(monkeypatch):
    monkeypatch.setenv("ACTIONS_ID_TOKEN_REQUEST_URL", "https://gh.example/oidc?x=1")
    monkeypatch.setenv("ACTIONS_ID_TOKEN_REQUEST_TOKEN", "request-token")


def test_request_github_oidc_token_appends_audience(monkeypatch):
    _set_oidc_env(monkeypatch)
    captured = {}

    def fake_request_json(request):
        captured["url"] = request.full_url
        captured["auth"] = request.headers.get("Authorization")
        return {"value": "the.jwt.token"}

    monkeypatch.setattr(oidc_token, "_request_json", fake_request_json)
    token = oidc_token.request_github_oidc_token("aud://relay")
    assert token == "the.jwt.token"
    assert "audience=aud%3A%2F%2Frelay" in captured["url"]
    assert captured["auth"] == "Bearer request-token"


def test_request_github_oidc_token_requires_env(monkeypatch):
    monkeypatch.delenv("ACTIONS_ID_TOKEN_REQUEST_URL", raising=False)
    with pytest.raises(ValidationError, match="ACTIONS_ID_TOKEN_REQUEST_URL"):
        oidc_token.request_github_oidc_token("aud")


def test_exchange_for_relay_token_accepts_camel_and_snake(monkeypatch):
    monkeypatch.setattr(
        oidc_token,
        "_request_json",
        lambda request: {"tokenType": "Bearer", "apiKey": "sk-clb-abc", "expiresAt": "2026-06-02T16:00:00Z"},
    )
    out = oidc_token.exchange_for_relay_token("jwt", oidc_token.DEFAULT_BROKER_URL)
    assert out == {"relay_token": "sk-clb-abc", "expires_at": "2026-06-02T16:00:00Z"}


def test_exchange_rejects_wrong_prefix(monkeypatch):
    monkeypatch.setattr(
        oidc_token,
        "_request_json",
        lambda request: {"tokenType": "Bearer", "apiKey": "sk-oai-nope", "expiresAt": "x"},
    )
    with pytest.raises(ValidationError, match="valid relay credential"):
        oidc_token.exchange_for_relay_token("jwt", oidc_token.DEFAULT_BROKER_URL)


def test_cli_oidc_relay_token_writes_outputs(monkeypatch, tmp_path, capsys):
    _set_oidc_env(monkeypatch)
    monkeypatch.setattr(
        oidc_token,
        "mint_relay_token",
        lambda audience, broker_url: {"relay_token": "sk-clb-xyz", "expires_at": "2026-06-02T16:00:00Z"},
    )
    output_file = tmp_path / "gh_output"
    monkeypatch.setenv("GITHUB_OUTPUT", str(output_file))
    rc = main(["oidc", "relay-token"])
    assert rc == 0
    written = output_file.read_text(encoding="utf-8")
    assert "relay_token=sk-clb-xyz" in written
    assert "expires_at=2026-06-02T16:00:00Z" in written
    assert "endpoint=https://relay-ai.dongwontuna.net/v1/responses" in written
    payload = json.loads(capsys.readouterr().out)
    assert payload["relay_token_minted"] is True
    assert payload["endpoint"] == "https://relay-ai.dongwontuna.net/v1/responses"
