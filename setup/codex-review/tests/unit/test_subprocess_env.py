from codex_review.security.subprocess_env import sanitized_env


def test_sanitized_env_removes_current_codex_app_secret_names(monkeypatch):
    monkeypatch.setenv("CODEX_APP_ID", "12345")
    monkeypatch.setenv("CODEX_APP_PRIVATE_KEY", "secret")

    env = sanitized_env()

    assert "CODEX_APP_ID" not in env
    assert "CODEX_APP_PRIVATE_KEY" not in env
