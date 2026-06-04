"""Provider-neutral model command adapter.

The workflow keeps repository write permissions out of model jobs. This adapter lets
repositories plug in any JSON-producing model runner without changing stage
contracts. The configured command receives the prompt and output path through
environment variables and must either write JSON to the output path or print JSON
on stdout.
"""
from __future__ import annotations

import json
import os
import re
import shlex
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, write_json
from codex_review.core.errors import ValidationError


def _stage_env_name(stage: str | None) -> str | None:
    if not stage:
        return None
    safe = re.sub(r"[^A-Za-z0-9]+", "_", stage).strip("_").upper()
    return f"CODEX_REVIEW_{safe}_MODEL_COMMAND" if safe else None


def configured_model_command(stage: str | None = None, explicit: str | None = None) -> str | None:
    """Return the stage-specific model command, falling back to the global one."""
    if explicit:
        return explicit
    stage_key = _stage_env_name(stage)
    if stage_key and os.environ.get(stage_key):
        return os.environ[stage_key]
    return os.environ.get("CODEX_REVIEW_MODEL_COMMAND")


def _parse_stdout_json(stdout: str) -> Any:
    text = stdout.strip()
    if not text:
        raise ValidationError("model command produced no JSON on stdout and did not write the output file")
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        # Some CLIs wrap JSON in logs. Accept the last complete-looking JSON object.
        start = text.rfind("{")
        if start == -1:
            raise
        return json.loads(text[start:])


def _validate_schema(payload: Any, expected_schema: str | None) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise ValidationError("model output must be a JSON object")
    if expected_schema and payload.get("schema_version") not in {None, expected_schema}:
        raise ValidationError(f"model output schema_version mismatch: expected {expected_schema}, got {payload.get('schema_version')}")
    if expected_schema:
        payload = dict(payload)
        payload.setdefault("schema_version", expected_schema)
    return payload




def _resolve_model_cwd(cwd: str | Path | None, target_repo_path: str | Path | None) -> Path | None:
    """Return the trusted working directory for model commands.

    Model commands may receive the PR-head repository path as data, but they must
    not execute *from* that untrusted checkout. If no explicit cwd is provided,
    prefer CODEX_REVIEW_MODEL_CWD and CODEX_REVIEW_TRUSTED_CHECKOUT.
    """
    selected = cwd or os.environ.get("CODEX_REVIEW_MODEL_CWD") or os.environ.get("CODEX_REVIEW_TRUSTED_CHECKOUT")
    resolved = Path(selected).resolve() if selected else None
    target = Path(target_repo_path or os.environ.get("CODEX_REVIEW_TARGET_REPO_PATH", "")).resolve() if (target_repo_path or os.environ.get("CODEX_REVIEW_TARGET_REPO_PATH")) else None
    if resolved and target and resolved == target:
        raise ValidationError("model command cwd must be a trusted checkout, not the PR-head target repo")
    if resolved and not resolved.exists():
        raise ValidationError(f"model command cwd does not exist: {resolved}")
    return resolved

def run_model_or_fallback(
    *,
    stage: str,
    prompt_path: str | Path | None,
    output_path: str | Path | None,
    expected_schema: str | None,
    fallback: dict[str, Any],
    model_command: str | None = None,
    timeout: int | None = None,
    cwd: str | Path | None = None,
    target_repo_path: str | Path | None = None,
) -> dict[str, Any]:
    """Run a configured model command, otherwise return a deterministic fallback.

    Environment supplied to the command:
      - CODEX_REVIEW_STAGE
      - CODEX_REVIEW_PROMPT_PATH
      - CODEX_REVIEW_OUTPUT_PATH
      - CODEX_REVIEW_EXPECTED_SCHEMA

    The command may use those variables directly. For convenience, if the command
    string contains ``{prompt}``, ``{output}``, ``{stage}``, or ``{schema}``, they
    are shell-quoted and interpolated before execution.
    """
    command = configured_model_command(stage, model_command)
    if not command:
        out = dict(fallback)
        out.setdefault("schema_version", expected_schema)
        out.setdefault("defaulted", True)
        return out

    if output_path is None:
        tmp = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False)
        tmp.close()
        output = Path(tmp.name)
    else:
        output = Path(output_path)
        output.parent.mkdir(parents=True, exist_ok=True)

    prompt = Path(prompt_path) if prompt_path else None
    if prompt and not prompt.exists():
        raise ValidationError(f"model prompt path does not exist: {prompt}")

    fmt = {
        "prompt": shlex.quote(prompt.as_posix() if prompt else ""),
        "output": shlex.quote(output.as_posix()),
        "stage": shlex.quote(stage),
        "schema": shlex.quote(expected_schema or ""),
    }
    if any(token in command for token in ("{prompt}", "{output}", "{stage}", "{schema}")):
        command = command.format(**fmt)

    env = os.environ.copy()
    target_repo = Path(target_repo_path).resolve() if target_repo_path else None
    model_cwd = _resolve_model_cwd(cwd, target_repo)
    env.update({
        "CODEX_REVIEW_STAGE": stage,
        "CODEX_REVIEW_PROMPT_PATH": prompt.as_posix() if prompt else "",
        "CODEX_REVIEW_OUTPUT_PATH": output.as_posix(),
        "CODEX_REVIEW_EXPECTED_SCHEMA": expected_schema or "",
        "CODEX_REVIEW_TARGET_REPO_PATH": target_repo.as_posix() if target_repo else os.environ.get("CODEX_REVIEW_TARGET_REPO_PATH", ""),
        "CODEX_REVIEW_MODEL_CWD": model_cwd.as_posix() if model_cwd else "",
    })
    argv = shlex.split(command)
    if not argv:
        raise ValidationError("model command is empty")
    proc = subprocess.run(
        argv,
        shell=False,
        cwd=model_cwd,
        env=env,
        text=True,
        capture_output=True,
        timeout=timeout or int(os.environ.get("CODEX_REVIEW_MODEL_TIMEOUT", "1800")),
    )
    if proc.returncode != 0:
        raise ValidationError(f"model command for {stage} failed with exit {proc.returncode}: {proc.stderr[-2000:]}")

    if output.exists() and output.stat().st_size > 0:
        payload = read_json(output)
    else:
        payload = _parse_stdout_json(proc.stdout)
        write_json(output, payload, expected_schema)
    return _validate_schema(payload, expected_schema)


def write_prompt_if_needed(prompt_text: str, out_path: str | Path | None, default_name: str = "prompt.md") -> Path:
    """Write prompt text next to the intended output artifact and return the path."""
    if out_path:
        base = Path(out_path)
        prompt_path = base.with_name(base.stem + ".prompt.md")
    else:
        prompt_path = Path(tempfile.mkdtemp(prefix="codex-review-prompt-")) / default_name
    prompt_path.parent.mkdir(parents=True, exist_ok=True)
    prompt_path.write_text(prompt_text, encoding="utf-8")
    return prompt_path
