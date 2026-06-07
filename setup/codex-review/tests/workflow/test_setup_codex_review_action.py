"""Contract tests for the setup-codex-review composite action.

The action installs the trusted codex-review helper from its own SHA-pinned
action source (the home-server-infra repo downloaded at the action ref). This
replaces the per-job pattern of checking out home-server-infra with a read
token and pip-installing ``trusted-core/setup/codex-review``. The action must
therefore: install from the bundled package (never a runtime checkout), expose
the bundled ``config.yml`` path, and never take a write token, App key, or
relay secret.
"""
from __future__ import annotations

from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
ACTION = ROOT / ".github" / "actions" / "setup-codex-review" / "action.yml"


def _load() -> dict:
    return yaml.safe_load(ACTION.read_text(encoding="utf-8"))


def _text() -> str:
    return ACTION.read_text(encoding="utf-8")


def test_action_exists_and_is_composite():
    data = _load()
    assert data["runs"]["using"] == "composite"


def test_action_installs_helper_from_bundled_source_not_runtime_checkout():
    text = _text()
    # Installs the bundled package located relative to the action source.
    assert "${GITHUB_ACTION_PATH}/../../.." in text
    assert "setup/codex-review" in text
    assert "python3 -m pip install" in text
    # Smoke the CLI so a broken install fails the step.
    assert "codex-review --help" in text
    # No runtime checkout of home-server-infra and no read token live here.
    assert "actions/checkout" not in text
    assert "repository: DongwonTTuna-Labs/home-server-infra" not in text
    assert "trusted-core/setup/codex-review" not in text


def test_action_exposes_config_path_output():
    data = _load()
    assert "config_path" in data["outputs"]
    assert "config.yml" in _text()


def test_action_takes_no_write_token_app_key_or_relay_secret():
    text = _text()
    for forbidden in (
        "CODEX_TRUSTED_CORE_READ_TOKEN",
        "CODEX_GITHUB_APP_ID",
        "CODEX_GITHUB_APP_PRIVATE_KEY",
        "GITHUB_TOKEN",
        "relay-token",
    ):
        assert forbidden not in text


def test_action_does_not_use_setup_python():
    # Home Server Runners ship python3; the repo never uses actions/setup-python.
    assert "actions/setup-python" not in _text()
