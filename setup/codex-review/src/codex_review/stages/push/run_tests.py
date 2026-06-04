"""Run allowlisted tests before push."""
from __future__ import annotations

import shlex
import subprocess
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError

from .safe_subprocess import sanitized_env


def _test_config(config: dict[str, Any]) -> dict[str, Any]:
    return config.get("tests", {}) if isinstance(config.get("tests"), dict) else {}


def _allowlist(config: dict[str, Any]) -> dict[str, str]:
    tc = _test_config(config)
    allow = tc.get("allowlist") or tc.get("commands") or {}
    if allow:
        return {str(k): str(v) for k, v in allow.items()}
    # Backward compatibility for trusted repository config only. These commands
    # are not accepted from model artifacts; they are mapped to generated IDs.
    return {f"default-{idx}": str(cmd) for idx, cmd in enumerate(tc.get("default_commands") or [], 1)}


def _default_test_ids(config: dict[str, Any]) -> list[str]:
    tc = _test_config(config)
    if tc.get("default_ids"):
        return [str(x) for x in tc.get("default_ids") or []]
    if tc.get("default_test_ids"):
        return [str(x) for x in tc.get("default_test_ids") or []]
    if tc.get("default_commands"):
        return [f"default-{idx}" for idx, _ in enumerate(tc.get("default_commands") or [], 1)]
    return []


def select_test_commands(merged_fix: dict[str, Any], config: dict[str, Any]) -> list[str]:
    """Resolve model-selected test IDs to trusted config commands.

    Model/fix artifacts may only request test identifiers. Raw shell commands in
    merged_fix["tests"] are rejected unless they exactly match an allowlist key.
    This prevents a model or PR-controlled artifact from smuggling shell into the
    pre-push test step.
    """
    allow = _allowlist(config)
    requested = merged_fix.get("test_ids") or merged_fix.get("tests") or _default_test_ids(config)
    ids = [str(x) for x in requested]
    commands: list[str] = []
    for test_id in ids:
        if test_id not in allow:
            raise ValidationError(f"unknown or non-allowlisted test id: {test_id}")
        commands.append(allow[test_id])
    return commands


def run_test_command(command: str, repo_path: str | Path, timeout: int = 600) -> dict[str, Any]:
    argv = shlex.split(command)
    if not argv:
        raise ValidationError("empty test command")
    proc = subprocess.run(argv, shell=False, cwd=Path(repo_path), text=True, capture_output=True, timeout=timeout, env=sanitized_env())
    return {
        "command": command,
        "argv": argv,
        "returncode": proc.returncode,
        "stdout": proc.stdout[-4000:],
        "stderr": proc.stderr[-4000:],
        "passed": proc.returncode == 0,
    }


def run_required_tests(commands: list[str], repo_path: str | Path) -> dict[str, Any]:
    results = [run_test_command(c, repo_path) for c in commands]
    return {"schema_version": "push-test-report.v1", "results": results, "passed": all(r["passed"] for r in results)}


def write_test_report(report: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, report, "push-test-report.v1")
