"""Workflow contract tests for explicit state artifact transport."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"


def _text(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def test_dispatch_payload_requires_and_forwards_state_pointer():
    text = _text("codex-loop-dispatch.yml")

    for field in ("state_run_id", "state_artifact_name"):
        upper = field.upper()
        assert f"PAYLOAD_{upper}: ${{{{ github.event.client_payload.{field} }}}}" in text
        assert f"require_non_empty {field} " in text
        assert re.search(rf"(?m)^      {field}: ", text)
        assert re.search(rf"(?m)^      {field}: \$\{{\{{ needs\.validate-payload\.outputs\.{field} \}}\}}$", text)

    assert "invalid-state_run_id" in text
    assert "invalid-state_artifact_name" in text


def test_reusable_workflow_uses_explicit_run_id_artifact_download():
    text = _text("codex-loop-reusable.yml")

    for field in ("state_run_id", "state_artifact_name"):
        assert re.search(rf"(?m)^      {field}:\s*$", text)
        assert f"required: true" in text[text.find(f"      {field}:") : text.find("      max_iterations:")]
        assert f"require_non_empty {field} " in text

    assert "uses: actions/download-artifact@v4" in text
    assert "run-id: ${{ inputs.state_run_id }}" in text
    assert "name: ${{ inputs.state_artifact_name }}" in text
    assert "codex-review loop read-state" in text
    assert "--loop-state \"codex-review-artifacts/prior-state/${INPUT_STATE_ARTIFACT_NAME}\"" in text


def test_manual_workflow_carries_state_pointer_to_reusable_core():
    text = _text("codex-loop-manual.yml")

    for field in ("state_run_id", "state_artifact_name"):
        assert re.search(rf"(?m)^      {field}:\s*$", text)
        assert f"{field}: ${{{{ inputs.{field} }}}}" in text


def test_active_workflows_do_not_use_head_sha_artifact_discovery():
    combined = "\n".join(_text(name) for name in ("codex-loop-dispatch.yml", "codex-loop-reusable.yml", "codex-loop-manual.yml"))

    forbidden = ("gh run list", "--headSha", "headSha")
    for marker in forbidden:
        assert marker not in combined
