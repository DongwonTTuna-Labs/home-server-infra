"""Tests for the Codex loop state-bundle transport contract and integrity verify."""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from codex_review.cli import main
from codex_review.loop.bundle import (
    ARTIFACT_MISSING,
    ARTIFACT_SCHEMA_INVALID,
    BUNDLE_CONTRACTS,
    DEFERRED_CARRY_FORWARD,
    infer_bundle_kind,
    verify_bundle,
)

STAGE_BUNDLE_KINDS = ("review", "design", "fix", "issue")


def _write_complete_bundle(directory: Path, kind: str) -> Path:
    directory.mkdir(parents=True, exist_ok=True)
    for index, fname in enumerate(BUNDLE_CONTRACTS[kind]["required_files"]):
        (directory / fname).write_text(json.dumps({"schema_version": f"{kind}.v1", "n": index}), encoding="utf-8")
    return directory


def test_bundle_contracts_cover_every_supported_stage_bundle_shape():
    assert set(BUNDLE_CONTRACTS) == {"review", "design", "fix", "issue", "loop-state"}
    for kind in STAGE_BUNDLE_KINDS:
        spec = BUNDLE_CONTRACTS[kind]
        assert spec["required_files"], kind
        assert spec["artifact_prefix"].startswith("codex-loop-")
        assert all(name.endswith(".json") for name in spec["required_files"]), kind


def test_infer_bundle_kind_maps_artifact_name_prefixes():
    assert infer_bundle_kind("codex-loop-review-state-corr-0") == "review"
    assert infer_bundle_kind("codex-loop-design-state-corr-1") == "design"
    assert infer_bundle_kind("codex-loop-fix-state-corr-2") == "fix"
    assert infer_bundle_kind("codex-loop-fix-push-corr-2") == "fix"
    assert infer_bundle_kind("codex-loop-issue-fallback-corr-3") == "issue"
    assert infer_bundle_kind("codex-loop-state-corr-4.json") == "loop-state"
    assert infer_bundle_kind("something-else") == "unknown"
    assert infer_bundle_kind("") == "unknown"
    assert infer_bundle_kind(None) == "unknown"


@pytest.mark.parametrize("kind", STAGE_BUNDLE_KINDS)
def test_verify_bundle_round_trip_accepts_complete_stage_bundle(tmp_path: Path, kind: str):
    bundle_dir = _write_complete_bundle(tmp_path / kind, kind)
    verdict = verify_bundle(bundle_dir, kind)
    assert verdict["ok"] is True
    assert verdict["terminal_reason"] == ""
    assert verdict["kind"] == kind
    assert set(verdict["present"]) == set(BUNDLE_CONTRACTS[kind]["required_files"])
    assert verdict["missing"] == []
    assert verdict["invalid"] == []


def test_verify_bundle_auto_infers_kind_from_artifact_name(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "design", "design")
    verdict = verify_bundle(bundle_dir, "auto", artifact_name="codex-loop-design-state-corr-9")
    assert verdict["kind"] == "design"
    assert verdict["ok"] is True


def test_verify_bundle_missing_required_file_routes_to_artifact_missing(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "review", "review")
    (bundle_dir / "route.json").unlink()
    verdict = verify_bundle(bundle_dir, "review")
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == ARTIFACT_MISSING == "artifact_missing"
    assert verdict["missing"] == ["route.json"]


def test_verify_bundle_missing_bundle_directory_is_artifact_missing(tmp_path: Path):
    verdict = verify_bundle(tmp_path / "never-downloaded", "fix")
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "artifact_missing"
    assert set(verdict["missing"]) == set(BUNDLE_CONTRACTS["fix"]["required_files"])


def test_verify_bundle_unknown_kind_is_artifact_missing_not_silent_success(tmp_path: Path):
    bundle_dir = tmp_path / "mystery"
    bundle_dir.mkdir()
    verdict = verify_bundle(bundle_dir, "auto", artifact_name="codex-loop-unknown-7")
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "artifact_missing"
    assert verdict["kind"] == "unknown"


def test_verify_bundle_rejects_corrupt_required_json_as_artifact_schema_invalid(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "design", "design")
    (bundle_dir / "chief-decision.json").write_text("not-json{", encoding="utf-8")
    verdict = verify_bundle(bundle_dir, "design")
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == ARTIFACT_SCHEMA_INVALID == "artifact_schema_invalid"
    assert verdict["invalid"] == ["chief-decision.json"]


def test_verify_bundle_missing_beats_invalid_when_both_present(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "fix", "fix")
    (bundle_dir / "merged-fix.json").write_text("broken{", encoding="utf-8")
    (bundle_dir / "validated-fix.json").unlink()
    verdict = verify_bundle(bundle_dir, "fix")
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "artifact_missing"


def test_verify_bundle_allows_initial_empty_bootstrap_only_when_requested(tmp_path: Path):
    empty_dir = tmp_path / "bootstrap"
    empty_dir.mkdir()

    blocked = verify_bundle(empty_dir, "loop-state")
    assert blocked["ok"] is False
    assert blocked["terminal_reason"] == "artifact_missing"

    bootstrapped = verify_bundle(empty_dir, "loop-state", allow_initial_empty=True)
    assert bootstrapped["ok"] is True
    assert bootstrapped["terminal_reason"] == ""
    assert bootstrapped["bootstrap"] is True


def test_partial_bundle_is_never_bootstrapped_and_stays_artifact_missing(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "review", "review")
    (bundle_dir / "techlead-decision.json").unlink()
    (bundle_dir / "publish-report.json").unlink()
    verdict = verify_bundle(bundle_dir, "review", allow_initial_empty=True)
    assert verdict["ok"] is False
    assert verdict["bootstrap"] is False
    assert verdict["terminal_reason"] == "artifact_missing"


def test_dispatch_ledger_is_required_in_continuation_capable_bundles():
    for kind in ("review", "design", "fix"):
        assert "dispatch-ledger.json" in BUNDLE_CONTRACTS[kind]["required_files"], kind
    assert "dispatch-ledger.json" not in BUNDLE_CONTRACTS["issue"]["required_files"]
    assert "dispatch-ledger.json" not in BUNDLE_CONTRACTS["loop-state"]["required_files"]
    assert DEFERRED_CARRY_FORWARD == ()


def test_verify_bundle_cli_round_trip_writes_verdict_artifact(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "review", "review")
    out = tmp_path / "verdict.json"
    rc = main(["loop", "verify-bundle", "--kind", "review", "--dir", str(bundle_dir), "--out", str(out)])
    assert rc == 0
    verdict = json.loads(out.read_text(encoding="utf-8"))
    assert verdict["ok"] is True
    assert verdict["schema_version"] == "loop-state-bundle-verify.v1"
    assert verdict["terminal_reason"] == ""


def test_verify_bundle_cli_reports_artifact_missing_for_incomplete_bundle(tmp_path: Path):
    bundle_dir = _write_complete_bundle(tmp_path / "design", "design")
    (bundle_dir / "design-plan.json").unlink()
    out = tmp_path / "verdict.json"
    rc = main([
        "loop", "verify-bundle",
        "--kind", "auto",
        "--name", "codex-loop-design-state-corr-0",
        "--dir", str(bundle_dir),
        "--out", str(out),
    ])
    assert rc == 0
    verdict = json.loads(out.read_text(encoding="utf-8"))
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "artifact_missing"
    assert verdict["kind"] == "design"
    assert "design-plan.json" in verdict["missing"]


def test_verify_bundle_cli_requires_dir(tmp_path: Path):
    rc = main(["loop", "verify-bundle", "--kind", "review"])
    assert rc == 2
