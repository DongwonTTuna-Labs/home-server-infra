"""Tests for artifact-backed repository_dispatch ledger guards."""
from __future__ import annotations

import json
from pathlib import Path

from codex_review.cli import main
from codex_review.loop.state import (
    append_dispatch_ledger_entry,
    build_dispatch_ledger_entry,
    empty_dispatch_ledger,
    evaluate_dispatch_ledger,
    read_dispatch_ledger_artifact,
)

CONFIG = str(Path(__file__).resolve().parents[2] / "config.yml")


def _payload(**overrides):
    payload = {
        "correlation_id": "corr-1",
        "stage": "review",
        "iteration": 1,
        "head_sha": "a" * 40,
        "state_run_id": "123",
        "state_artifact_name": "codex-loop-review-state-corr-1-0",
        "max_iterations": 5,
    }
    payload.update(overrides)
    return payload


def test_dispatch_ledger_entry_uses_logical_key_not_json_bytes():
    left = build_dispatch_ledger_entry(_payload(extra="ignored"))
    right = build_dispatch_ledger_entry({"next_stage": "review", **_payload(state_run_id="999")})
    assert (left["correlation_id"], left["stage"], left["iteration"], left["head_sha"]) == (
        right["correlation_id"], right["stage"], right["iteration"], right["head_sha"]
    )


def test_dispatch_guard_allows_self_staged_candidate_in_state_bundle():
    ledger = append_dispatch_ledger_entry(None, _payload(status="staged"))
    verdict = evaluate_dispatch_ledger(ledger, _payload(), max_iterations=5, dispatch_cap=20)
    assert verdict["ok"] is True


def test_dispatch_guard_blocks_one_existing_identical_entry_before_live_dispatch():
    ledger = append_dispatch_ledger_entry(None, _payload())
    verdict = evaluate_dispatch_ledger(ledger, _payload(), max_iterations=5, dispatch_cap=20)
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "dispatch_duplicate"


def test_dispatch_guard_blocks_duplicate_before_live_dispatch():
    ledger = append_dispatch_ledger_entry(None, _payload())
    ledger = append_dispatch_ledger_entry(ledger, _payload(state_run_id="124"))
    verdict = evaluate_dispatch_ledger(ledger, _payload(), max_iterations=5, dispatch_cap=20)
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "dispatch_duplicate"


def test_dispatch_guard_uses_iteration_greater_equal_max_iterations():
    verdict = evaluate_dispatch_ledger({}, _payload(iteration=5), max_iterations=5, dispatch_cap=20)
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "max_iterations"


def test_dispatch_guard_enforces_per_correlation_cap_as_max_iterations():
    ledger = None
    for iteration in range(2):
        ledger = append_dispatch_ledger_entry(ledger, _payload(iteration=iteration, stage=f"stage-{iteration}"))
    verdict = evaluate_dispatch_ledger(ledger, _payload(iteration=3, stage="issue"), max_iterations=5, dispatch_cap=2)
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "max_iterations"


def test_dispatch_guard_detects_repeated_stage_head_signature_as_oscillation():
    ledger = None
    ledger = append_dispatch_ledger_entry(ledger, _payload(iteration=1))
    ledger = append_dispatch_ledger_entry(ledger, _payload(iteration=2))
    verdict = evaluate_dispatch_ledger(ledger, _payload(iteration=3), max_iterations=5, dispatch_cap=20)
    assert verdict["ok"] is False
    assert verdict["terminal_reason"] == "oscillation_detected"


def test_cli_guard_dispatch_reads_artifact_ledger(tmp_path: Path):
    ledger_path = tmp_path / "dispatch-ledger.json"
    payload_path = tmp_path / "payload.json"
    out_path = tmp_path / "guard.json"
    ledger_path.write_text(json.dumps(empty_dispatch_ledger()), encoding="utf-8")
    payload_path.write_text(json.dumps(_payload()), encoding="utf-8")
    rc = main(["loop", "guard-dispatch", "--config", CONFIG, "--in", str(payload_path), "--ledger", str(ledger_path), "--out", str(out_path)])
    assert rc == 0
    verdict = json.loads(out_path.read_text(encoding="utf-8"))
    assert verdict["schema_version"] == "dispatch-guard.v1"
    assert verdict["ok"] is True


def test_cli_read_state_imports_dispatch_ledger_sidecar(tmp_path: Path):
    state_path = tmp_path / "state.json"
    ledger_path = tmp_path / "dispatch-ledger.json"
    out_path = tmp_path / "loop-state.json"
    state_path.write_text(json.dumps({"schema_version": "loop-state.v1"}), encoding="utf-8")
    ledger_path.write_text(json.dumps(append_dispatch_ledger_entry(None, _payload())), encoding="utf-8")
    rc = main(["loop", "read-state", "--loop-state", str(state_path), "--ledger", str(ledger_path), "--out", str(out_path)])
    assert rc == 0
    state = json.loads(out_path.read_text(encoding="utf-8"))
    assert state["dispatch_ledger"][0]["correlation_id"] == "corr-1"


def test_read_dispatch_ledger_accepts_loop_state_payload(tmp_path: Path):
    state_path = tmp_path / "loop-state.json"
    state_path.write_text(json.dumps({"schema_version": "loop-state.v1", "dispatch_ledger": [build_dispatch_ledger_entry(_payload())]}), encoding="utf-8")
    ledger = read_dispatch_ledger_artifact(state_path)
    assert ledger["entries"][0]["stage"] == "review"
