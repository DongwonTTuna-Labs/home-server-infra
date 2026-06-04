from pathlib import Path

from codex_review.cli import _preferred_artifact_paths


def test_preferred_artifact_paths_use_validated_outputs_once(tmp_path: Path):
    artifact = tmp_path / "codex-review-review-correctness"
    artifact.mkdir()
    raw = artifact / "findings.json"
    validated = artifact / "findings.validated.json"
    raw.write_text("{}", encoding="utf-8")
    validated.write_text("{}", encoding="utf-8")

    assert _preferred_artifact_paths([str(tmp_path)], primary="findings.validated.json", fallback="findings.json") == [str(validated)]


def test_preferred_artifact_paths_fall_back_to_raw_outputs(tmp_path: Path):
    artifact = tmp_path / "codex-review-review-correctness"
    artifact.mkdir()
    raw = artifact / "findings.json"
    raw.write_text("{}", encoding="utf-8")

    assert _preferred_artifact_paths([str(tmp_path)], primary="findings.validated.json", fallback="findings.json") == [str(raw)]
