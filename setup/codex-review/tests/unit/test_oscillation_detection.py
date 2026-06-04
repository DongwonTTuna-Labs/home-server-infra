"""Tests for autofix loop oscillation / round-cap detection (PR2/A)."""
from __future__ import annotations

import json
from pathlib import Path

from codex_review.cli import main
from codex_review.loop.state import (
    append_push_to_loop_state,
    build_push_entry,
    detect_oscillation,
    fingerprint_patch,
    normalized_finding_keys_from_plan,
    read_loop_state_artifact,
    read_loop_state_payload,
)

CONFIG = str(Path(__file__).resolve().parents[2] / "config.yml")
CFG = {"autofix": {"max_rounds": 5, "pingpong_threshold": 2, "revert_threshold": 0.8}}


def test_fingerprint_patch_normalizes_whitespace_and_tracks_paths():
    patch = "diff --git a/src/a.rs b/src/a.rs\n@@ -1,2 +1,2 @@\n-  old_line\n+new_line\n+   new_line\n"
    fp = fingerprint_patch(patch)
    assert fp["touched_paths"] == ["src/a.rs"]
    # "+new_line" and "+   new_line" normalize to the same line -> one fingerprint
    assert len(fp["added_line_fp"]) == 1
    assert len(fp["removed_line_fp"]) == 1


def test_normalized_finding_keys_from_plan():
    plan = {"edit_sequence": [{"task_id": "OpenSpec-Fallback-1", "finding_ids": ["F_Nonce", "F2"]}]}
    assert normalized_finding_keys_from_plan(plan) == ["f2", "f_nonce", "openspec-fallback-1"]


def test_build_push_entry_empty_patch_has_falsy_hash():
    entry = build_push_entry(1, {"commit_sha": "c1", "head_sha": "s1"}, "", {})
    assert entry["patch_sha256"] == ""  # empty patch must not be a matchable hash
    assert entry["added_line_fp"] == []


def test_append_trims_to_window_and_counts_rounds():
    state = {}
    for i in range(4):
        state = append_push_to_loop_state(state, {"round": i + 1, "head_sha": f"s{i}"}, window=3)
    assert state["round_count"] == 4
    assert len(state["recent_pushes"]) == 3  # window trim
    assert state["recent_pushes"][0]["round"] == 2  # oldest dropped


def test_detect_ok_with_no_history():
    cand = build_push_entry(1, {}, "diff --git a/x b/x\n@@ -1 +1 @@\n+a\n", {})
    assert detect_oscillation({}, cand, CFG)["ok"] is True


def test_detect_max_rounds():
    verdict = detect_oscillation({"round_count": 5, "recent_pushes": []}, {"patch_sha256": "h"}, CFG)
    assert verdict["ok"] is False and verdict["status"] == "max_rounds_reached"


def test_detect_exact_patch_repeat():
    prior = {"round_count": 1, "recent_pushes": [{"patch_sha256": "HASH1"}]}
    verdict = detect_oscillation(prior, {"patch_sha256": "HASH1", "normalized_finding_keys": [], "added_line_fp": [], "touched_paths": []}, CFG)
    assert verdict["ok"] is False and verdict["status"] == "oscillation_detected"


def test_detect_finding_key_pingpong():
    prior = {"round_count": 2, "recent_pushes": [{"normalized_finding_keys": ["k_nonce"]}, {"normalized_finding_keys": ["k_nonce"]}]}
    cand = {"patch_sha256": "new", "normalized_finding_keys": ["k_nonce"], "added_line_fp": [], "touched_paths": []}
    verdict = detect_oscillation(prior, cand, CFG)
    assert verdict["ok"] is False and "ping-pong" in verdict["reason"]


def test_detect_revert_similarity():
    prior = {"round_count": 1, "recent_pushes": [{"removed_line_fp": ["fp1", "fp2"], "touched_paths": ["src/a.rs"]}]}
    cand = {"patch_sha256": "new", "normalized_finding_keys": [], "added_line_fp": ["fp1", "fp2"], "touched_paths": ["src/a.rs"]}
    verdict = detect_oscillation(prior, cand, CFG)
    assert verdict["ok"] is False and "revert" in verdict["reason"]


def test_detect_revert_ignored_when_different_paths():
    prior = {"round_count": 1, "recent_pushes": [{"removed_line_fp": ["fp1", "fp2"], "touched_paths": ["src/other.rs"]}]}
    cand = {"patch_sha256": "new", "normalized_finding_keys": [], "added_line_fp": ["fp1", "fp2"], "touched_paths": ["src/a.rs"]}
    assert detect_oscillation(prior, cand, CFG)["ok"] is True


def test_read_loop_state_from_artifact_json(tmp_path):
    state_path = tmp_path / "loop-state.json"
    state_path.write_text(json.dumps({"schema_version": "loop-state.v1", "round_count": 2, "recent_pushes": [{"round": 1}]}), encoding="utf-8")
    state = read_loop_state_artifact(state_path)
    assert state["schema_version"] == "loop-state.v1"
    assert state["round_count"] == 2
    assert state["recent_pushes"] == [{"round": 1}]


def test_read_loop_state_from_reentry_payload_object():
    state = read_loop_state_payload({"schema_version": "reentry-loop-state.v1", "loop_state": {"round_count": 3, "recent_pushes": [{"round": 3}]}})
    assert state["schema_version"] == "loop-state.v1"
    assert state["round_count"] == 3
    assert state["recent_pushes"] == [{"round": 3}]


def test_read_loop_state_missing_artifact_defaults_empty(tmp_path):
    state = read_loop_state_artifact(tmp_path / "missing.json")
    assert state == {"schema_version": "loop-state.v1", "recent_pushes": [], "round_count": 0, "dispatch_ledger": []}


# ---- CLI wiring ----

def test_cli_check_loop_budget_downgrades_on_pingpong(tmp_path):
    (tmp_path / "validated.json").write_text(json.dumps({"schema_version": "push-validated-fix.v1", "status": "validated", "validated": True, "head_sha": "s2"}), encoding="utf-8")
    (tmp_path / "merged.json").write_text(json.dumps({"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": "diff --git a/x b/x\n@@ -1 +1 @@\n+changed"}), encoding="utf-8")
    (tmp_path / "plan.json").write_text(json.dumps({"schema_version": "design-plan.v1", "edit_sequence": [{"task_id": "t1", "finding_ids": ["F_nonce"]}]}), encoding="utf-8")
    (tmp_path / "prior.json").write_text(json.dumps({"schema_version": "loop-state.v1", "round_count": 2, "recent_pushes": [{"normalized_finding_keys": ["f_nonce"]}, {"normalized_finding_keys": ["f_nonce"]}]}), encoding="utf-8")
    out = tmp_path / "out.json"
    rc = main(["push", "check-loop-budget", "--config", CONFIG, "--in", str(tmp_path / "validated.json"), "--result", str(tmp_path / "merged.json"), "--design-plan", str(tmp_path / "plan.json"), "--loop-state", str(tmp_path / "prior.json"), "--out", str(out)])
    assert rc == 0
    result = json.loads(out.read_text(encoding="utf-8"))
    assert result["loop_budget_ok"] is False
    assert result["status"] == "oscillation_detected"
    assert result["validated"] is False


def test_cli_check_loop_budget_passthrough_when_not_validated(tmp_path):
    (tmp_path / "validated.json").write_text(json.dumps({"schema_version": "push-validated-fix.v1", "status": "tests_failed", "validated": False}), encoding="utf-8")
    out = tmp_path / "out.json"
    rc = main(["push", "check-loop-budget", "--config", CONFIG, "--in", str(tmp_path / "validated.json"), "--out", str(out)])
    assert rc == 0
    result = json.loads(out.read_text(encoding="utf-8"))
    assert result["loop_budget_ok"] is True and result["status"] == "tests_failed"


def test_cli_read_state_loads_artifact_json(tmp_path):
    state_path = tmp_path / "prior.json"
    state_path.write_text(json.dumps({"schema_version": "loop-state.v1", "round_count": 2, "recent_pushes": [{"round": 2, "head_sha": "s2"}]}), encoding="utf-8")
    out = tmp_path / "loop-state.json"
    rc = main(["loop", "read-state", "--loop-state", str(state_path), "--out", str(out)])
    assert rc == 0
    state = json.loads(out.read_text(encoding="utf-8"))
    assert state["round_count"] == 2
    assert state["recent_pushes"] == [{"round": 2, "head_sha": "s2"}]


def test_cli_read_state_ignores_comment_path_even_with_pr_context_and_token(tmp_path, monkeypatch):
    import codex_review.github.comments as comments

    def fail_comment_read(*args, **kwargs):
        raise AssertionError("comment-backed loop state must not be read")

    monkeypatch.setattr(comments, "list_issue_comments", fail_comment_read)
    pr = tmp_path / "pr.json"
    pr.write_text(json.dumps({"owner": "o", "repo": "r", "pr_number": 1}), encoding="utf-8")
    out = tmp_path / "loop-state.json"
    rc = main(["loop", "read-state", "--pr-context", str(pr), "--token", "not-used", "--out", str(out)])
    assert rc == 0
    state = json.loads(out.read_text(encoding="utf-8"))
    assert state == {"schema_version": "loop-state.v1", "recent_pushes": [], "round_count": 0, "dispatch_ledger": []}


def test_cli_record_push_emits_loop_state_artifact_json(tmp_path):
    (tmp_path / "push.json").write_text(json.dumps({"schema_version": "push-result.v1", "status": "pushed", "pushed": True, "commit_sha": "c1", "head_sha": "s1"}), encoding="utf-8")
    (tmp_path / "merged.json").write_text(json.dumps({"schema_version": "fix-merge-merged-fix.v1", "patch": "diff --git a/x b/x\n@@ -1 +1 @@\n+changed"}), encoding="utf-8")
    (tmp_path / "prior.json").write_text(json.dumps({"schema_version": "loop-state.v1", "round_count": 1, "recent_pushes": [{"round": 1, "head_sha": "old"}]}), encoding="utf-8")
    out = tmp_path / "next-loop-state.json"
    rc = main(["push", "record-push", "--config", CONFIG, "--in", str(tmp_path / "push.json"), "--result", str(tmp_path / "merged.json"), "--loop-state", str(tmp_path / "prior.json"), "--out", str(out)])
    assert rc == 0
    state = json.loads(out.read_text(encoding="utf-8"))
    assert state["schema_version"] == "loop-state.v1"
    assert state["round_count"] == 2
    assert [entry["round"] for entry in state["recent_pushes"]] == [1, 2]
    assert state["recent_pushes"][-1]["commit_sha"] == "c1"


def test_cli_record_push_does_not_call_comment_writer(tmp_path, monkeypatch):
    import codex_review.github.comments as comments

    def fail_comment_write(*args, **kwargs):
        raise AssertionError("comment-backed loop state must not be written")

    monkeypatch.setattr(comments, "upsert_sticky_comment", fail_comment_write)
    (tmp_path / "push.json").write_text(json.dumps({"schema_version": "push-result.v1", "status": "pushed", "pushed": True, "commit_sha": "c1", "head_sha": "s1"}), encoding="utf-8")
    (tmp_path / "merged.json").write_text(json.dumps({"schema_version": "fix-merge-merged-fix.v1", "patch": "diff --git a/x b/x\n@@ -1 +1 @@\n+changed"}), encoding="utf-8")
    (tmp_path / "pr.json").write_text(json.dumps({"owner": "o", "repo": "r", "pr_number": 1}), encoding="utf-8")
    out = tmp_path / "next-loop-state.json"
    rc = main(["push", "record-push", "--config", CONFIG, "--in", str(tmp_path / "push.json"), "--result", str(tmp_path / "merged.json"), "--pr-context", str(tmp_path / "pr.json"), "--token", "not-used", "--out", str(out)])
    assert rc == 0
    state = json.loads(out.read_text(encoding="utf-8"))
    assert state["round_count"] == 1


def test_cli_read_state_empty_without_token(tmp_path):
    out = tmp_path / "loop-state.json"
    rc = main(["loop", "read-state", "--out", str(out)])
    assert rc == 0
    state = json.loads(out.read_text(encoding="utf-8"))
    assert state["recent_pushes"] == [] and state["round_count"] == 0 and state["dispatch_ledger"] == []
