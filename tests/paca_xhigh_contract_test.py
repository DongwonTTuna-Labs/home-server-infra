from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PATCHER = REPO_ROOT / "stacks" / "paca" / "overrides" / "ai-agent" / "enforce_xhigh.py"


def run_patcher(builder: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [sys.executable, str(PATCHER)],
        check=False,
        capture_output=True,
        env=os.environ | {"PACA_BUILDER_PATH": str(builder)},
        text=True,
    )


def test_adds_xhigh_when_builder_matches_contract(tmp_path: Path) -> None:
    # Given
    builder = tmp_path / "builder.py"
    _ = builder.write_text(
        "return LLM(\n        model=model_str,\n        stream=True,\n    )\n",
        encoding="utf-8",
    )

    # When
    result = run_patcher(builder)

    # Then
    assert result.returncode == 0
    assert 'reasoning_effort="xhigh"' in builder.read_text(encoding="utf-8")


def test_is_idempotent_when_xhigh_is_already_present(tmp_path: Path) -> None:
    # Given
    builder = tmp_path / "builder.py"
    source = (
        "return LLM(\n"
        "        model=model_str,\n"
        "        stream=True,\n"
        '        reasoning_effort="xhigh",\n'
        "    )\n"
    )
    _ = builder.write_text(source, encoding="utf-8")

    # When
    result = run_patcher(builder)

    # Then
    assert result.returncode == 0
    assert builder.read_text(encoding="utf-8") == source


def test_fails_closed_when_upstream_builder_drifts(tmp_path: Path) -> None:
    # Given
    builder = tmp_path / "builder.py"
    source = "return LLM(model=model_str, stream=True)\n"
    _ = builder.write_text(source, encoding="utf-8")

    # When
    result = run_patcher(builder)

    # Then
    assert result.returncode != 0
    assert builder.read_text(encoding="utf-8") == source
