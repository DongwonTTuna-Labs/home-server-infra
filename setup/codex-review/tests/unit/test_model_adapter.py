import json
import sys
from pathlib import Path

from codex_review.model.adapter import run_model_or_fallback


def test_model_adapter_uses_fallback_without_command(monkeypatch, tmp_path):
    monkeypatch.delenv("CODEX_REVIEW_MODEL_COMMAND", raising=False)
    out = run_model_or_fallback(stage="review", prompt_path=None, output_path=tmp_path/"out.json", expected_schema="x.v1", fallback={"items": []})
    assert out["schema_version"] == "x.v1"
    assert out["defaulted"] is True


def test_model_adapter_runs_command_and_reads_output(monkeypatch, tmp_path):
    prompt = tmp_path / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    writer = tmp_path / "writer.py"
    writer.write_text(
        "import json, os\n"
        "open(os.environ['CODEX_REVIEW_OUTPUT_PATH'], 'w', encoding='utf-8').write(json.dumps({'schema_version': os.environ['CODEX_REVIEW_EXPECTED_SCHEMA'], 'ok': True}))\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("CODEX_REVIEW_MODEL_COMMAND", f"{sys.executable} {writer}")
    out = run_model_or_fallback(stage="review", prompt_path=prompt, output_path=tmp_path/"out.json", expected_schema="review-axis-findings.v1", fallback={})
    assert out["ok"] is True
    assert out["schema_version"] == "review-axis-findings.v1"


def test_model_adapter_runs_from_trusted_cwd_and_exposes_target_repo(monkeypatch, tmp_path):
    trusted = tmp_path / "trusted"
    target = tmp_path / "pr-head"
    trusted.mkdir()
    target.mkdir()
    prompt = trusted / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    writer = trusted / "writer.py"
    writer.write_text(
        "import json, os, pathlib\n"
        "payload = {\n"
        "  'schema_version': os.environ['CODEX_REVIEW_EXPECTED_SCHEMA'],\n"
        "  'cwd': pathlib.Path.cwd().as_posix(),\n"
        "  'target': os.environ.get('CODEX_REVIEW_TARGET_REPO_PATH'),\n"
        "}\n"
        "open(os.environ['CODEX_REVIEW_OUTPUT_PATH'], 'w', encoding='utf-8').write(json.dumps(payload))\n",
        encoding="utf-8",
    )
    monkeypatch.setenv("CODEX_REVIEW_MODEL_COMMAND", f"{sys.executable} writer.py")
    out = run_model_or_fallback(
        stage="fix_dispatch_task",
        prompt_path=prompt,
        output_path=tmp_path / "out.json",
        expected_schema="fix-dispatch-agent-result.v1",
        fallback={},
        cwd=trusted,
        target_repo_path=target,
    )
    assert out["cwd"] == trusted.as_posix()
    assert out["target"] == target.resolve().as_posix()


def test_model_adapter_rejects_pr_head_as_model_cwd(monkeypatch, tmp_path):
    from codex_review.core.errors import ValidationError

    target = tmp_path / "pr-head"
    target.mkdir()
    prompt = tmp_path / "prompt.md"
    prompt.write_text("prompt", encoding="utf-8")
    monkeypatch.setenv("CODEX_REVIEW_MODEL_COMMAND", f"{sys.executable} -c 'print({{}})'")
    try:
        run_model_or_fallback(
            stage="fix_dispatch_task",
            prompt_path=prompt,
            output_path=tmp_path / "out.json",
            expected_schema="fix-dispatch-agent-result.v1",
            fallback={},
            cwd=target,
            target_repo_path=target,
        )
    except ValidationError as exc:
        assert "trusted checkout" in str(exc)
    else:
        raise AssertionError("expected target repo cwd to be rejected")
