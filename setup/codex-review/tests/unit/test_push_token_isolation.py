import os

import pytest

from codex_review.stages.push.run_tests import run_required_tests, select_test_commands
from codex_review.core.errors import ValidationError


def test_model_tests_must_be_allowlisted_ids():
    with pytest.raises(ValidationError):
        select_test_commands({"tests": ["echo leaked $GITHUB_TOKEN"]}, {"tests": {"allowlist": {"safe": "python -V"}}})


def test_test_runner_strips_github_tokens(tmp_path, monkeypatch):
    script = tmp_path / "check_env.py"
    script.write_text("import os, sys\nsys.exit(1 if os.environ.get('GITHUB_TOKEN') else 0)\n", encoding="utf-8")
    monkeypatch.setenv("GITHUB_TOKEN", "secret-token")
    report = run_required_tests([f"python {script}"], tmp_path)
    assert report["passed"] is True


def test_allowlisted_test_ids_resolve_to_trusted_commands():
    commands = select_test_commands({"test_ids": ["unit"]}, {"tests": {"allowlist": {"unit": "python -V"}}})
    assert commands == ["python -V"]
