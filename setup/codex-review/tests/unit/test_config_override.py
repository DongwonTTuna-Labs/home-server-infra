"""Tests for consumer-overridable config: generic core + layered overrides (Task 8)."""
from __future__ import annotations

from pathlib import Path

import pytest

import codex_review.core.config as config_module
from codex_review.core.config import CONFIG_OVERRIDE_ENV, DEFAULT_CONFIG, load_config
from codex_review.core.errors import ValidationError

SETUP_ROOT = Path(__file__).resolve().parents[2]
CORE_CONFIG = SETUP_ROOT / "config.yml"
CONSUMER_OVERRIDE = SETUP_ROOT / "tests" / "fixtures" / "config" / "consumer_override.yml"


def test_core_config_ships_generic_defaults_only():
    config = load_config()
    assert config["trusted"]["user"] == ""
    assert config["trusted"]["codex_review_authors"] == []
    assert config["autofix"]["forbidden_files"] == []
    keywords = config["autofix"]["dangerous_keywords"]
    for venue_specific in ["calldata", "eip-712", "wallet", "wallet-create", "wire", "serde"]:
        assert venue_specific not in keywords
    assert config["tests"].get("allowlist") in ({}, None)


def test_core_config_file_has_no_repo_specific_values():
    text = CORE_CONFIG.read_text(encoding="utf-8")
    for repo_specific in ["DongwonTTuna", "Cargo.toml", "Cargo.lock", "cargo ", "codex-reviewer-for-dongwonttuna"]:
        assert repo_specific not in text


def test_override_path_deep_merges_nested_values():
    config = load_config(override_path=CONSUMER_OVERRIDE)
    assert config["trusted"]["user"] == "DongwonTTuna"
    assert "codex-reviewer-for-dongwonttuna" in config["trusted"]["codex_review_authors"]
    assert config["autofix"]["forbidden_files"] == ["Cargo.toml", "Cargo.lock"]
    assert "calldata" in config["autofix"]["dangerous_keywords"]
    assert config["tests"]["allowlist"]["cargo_test"] == "cargo test --workspace --all-features"
    assert config["review"]["axes"] == DEFAULT_CONFIG["review"]["axes"]
    # The consumer override does not set max_rounds, so it stays at the core value.
    assert config["autofix"]["max_rounds"] == load_config()["autofix"]["max_rounds"]


def test_override_mapping_takes_highest_precedence():
    config = load_config(
        override_path=CONSUMER_OVERRIDE,
        override={"trusted": {"user": "other-actor"}, "autofix": {"max_rounds": 9}},
    )
    assert config["trusted"]["user"] == "other-actor"
    assert "codex-reviewer-for-dongwonttuna" in config["trusted"]["codex_review_authors"]
    assert config["autofix"]["max_rounds"] == 9
    assert config["autofix"]["forbidden_files"] == ["Cargo.toml", "Cargo.lock"]


def test_env_var_override_merges_when_file_exists(monkeypatch):
    monkeypatch.setenv(CONFIG_OVERRIDE_ENV, str(CONSUMER_OVERRIDE))
    config = load_config()
    assert config["trusted"]["user"] == "DongwonTTuna"
    assert config["autofix"]["forbidden_files"] == ["Cargo.toml", "Cargo.lock"]


def test_missing_env_override_falls_back_to_generic_defaults(monkeypatch):
    monkeypatch.setenv(CONFIG_OVERRIDE_ENV, str(SETUP_ROOT / "does-not-exist.yml"))
    config = load_config()
    assert config["trusted"]["user"] == ""
    assert config["autofix"]["forbidden_files"] == []
    assert config["review"]["axes"]


def test_no_override_yields_generic_config(monkeypatch):
    monkeypatch.delenv(CONFIG_OVERRIDE_ENV, raising=False)
    config = load_config()
    assert config["trusted"]["codex_review_authors"] == []
    assert config["autofix"]["forbidden_files"] == []


def test_explicit_override_path_missing_raises():
    with pytest.raises(ValidationError, match="missing config override file"):
        load_config(override_path=SETUP_ROOT / "does-not-exist.yml")


def test_override_mapping_must_be_mapping():
    with pytest.raises(ValidationError, match="config override must be a mapping"):
        load_config(override=[1, 2, 3])  # type: ignore[arg-type]


def test_missing_base_config_falls_back_to_generic(monkeypatch, tmp_path):
    monkeypatch.setattr(config_module, "setup_root", lambda: tmp_path)
    monkeypatch.delenv(CONFIG_OVERRIDE_ENV, raising=False)
    config = load_config()
    assert config["base_branch"] == DEFAULT_CONFIG["base_branch"]
    assert config["autofix"]["forbidden_files"] == []
    assert config["review"]["axes"] == DEFAULT_CONFIG["review"]["axes"]


def test_explicit_missing_path_still_errors(tmp_path):
    with pytest.raises(ValidationError, match="missing config file"):
        load_config(tmp_path / "nope.yml")


def test_repo_specific_paths_not_required_for_normal_load():
    config = load_config()
    assert not config["autofix"]["forbidden_files"]
    config_module.validate_config(config)


def test_config_load_requires_no_secrets(monkeypatch):
    for secret_env in ["GITHUB_TOKEN", "CODEX_GITHUB_APP_PRIVATE_KEY", "CODEX_GITHUB_APP_ID"]:
        monkeypatch.delenv(secret_env, raising=False)
    monkeypatch.delenv(CONFIG_OVERRIDE_ENV, raising=False)
    config = load_config()
    flattened = repr(config).lower()
    for secret_marker in ["private_key", "-----begin", "app_private", "ghp_", "bearer "]:
        assert secret_marker not in flattened
