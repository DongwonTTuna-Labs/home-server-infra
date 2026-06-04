import json
import pytest

from codex_review.stages.fix_dispatch.collect import collect_agent_results


def test_collect_agent_results_adds_safe_missing_defaults(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}, {"task_id":"t2"}]}
    p = tmp_path / "r1.json"
    p.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"no_safe_fix"}), encoding="utf-8")
    out = collect_agent_results(manifest, [p])
    assert out["ready_for_merge"] is False
    assert out["missing_task_ids"] == ["t2"]
    assert [r["task_id"] for r in out["results"]] == ["t1", "t2"]


def test_collect_agent_results_rejects_unknown_task(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}]}
    p = tmp_path / "bad.json"
    p.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"other", "status":"no_safe_fix"}), encoding="utf-8")
    with pytest.raises(Exception):
        collect_agent_results(manifest, [p])


def test_collect_agent_results_prefers_validated_result_over_raw_duplicate(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}]}
    task_dir = tmp_path / "codex-review-fix_dispatch-agent-t1"
    task_dir.mkdir()
    raw = task_dir / "result.json"
    validated = task_dir / "result.validated.json"
    raw.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"no_safe_fix"}), encoding="utf-8")
    validated.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"patched", "patch":"diff --git a/docs/x b/docs/x\n"}), encoding="utf-8")

    out = collect_agent_results(manifest, [raw, validated])

    assert out["ready_for_merge"] is True
    assert out["missing_task_ids"] == []
    assert [r["status"] for r in out["results"]] == ["patched"]


def test_collect_agent_results_keeps_raw_only_results_when_no_validated_result(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}, {"task_id":"t2"}]}
    t1 = tmp_path / "codex-review-fix_dispatch-agent-t1"
    t2 = tmp_path / "codex-review-fix_dispatch-agent-t2"
    t1.mkdir()
    t2.mkdir()
    raw1 = t1 / "result.json"
    validated1 = t1 / "result.validated.json"
    raw2 = t2 / "result.json"
    raw1.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"no_safe_fix"}), encoding="utf-8")
    validated1.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"patched", "patch":"diff --git a/docs/x b/docs/x\n"}), encoding="utf-8")
    raw2.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t2", "status":"no_safe_fix"}), encoding="utf-8")

    out = collect_agent_results(manifest, [raw1, validated1, raw2])

    assert out["missing_task_ids"] == []
    assert [(r["task_id"], r["status"]) for r in out["results"]] == [("t1", "patched"), ("t2", "no_safe_fix")]


def test_collect_agent_results_ignores_task_metadata_json(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}]}
    task_dir = tmp_path / "codex-review-fix_dispatch-agent-t1"
    task_dir.mkdir()
    task_metadata = task_dir / "task.json"
    validated = task_dir / "result.validated.json"
    task_metadata.write_text(json.dumps({"task_id":"t1", "allowed_files":["docs/x"]}), encoding="utf-8")
    validated.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"patched", "patch":"diff --git a/docs/x b/docs/x\n"}), encoding="utf-8")

    out = collect_agent_results(manifest, [task_metadata, validated])

    assert out["missing_task_ids"] == []
    assert [r["status"] for r in out["results"]] == ["patched"]


def test_collect_agent_results_rejects_mixed_priority_duplicates_from_different_dirs(tmp_path):
    manifest = {"tasks": [{"task_id":"t1"}]}
    stale_dir = tmp_path / "codex-review-fix_dispatch-agent-t1-stale"
    current_dir = tmp_path / "codex-review-fix_dispatch-agent-t1-current"
    stale_dir.mkdir()
    current_dir.mkdir()
    stale_raw = stale_dir / "result.json"
    current_validated = current_dir / "result.validated.json"
    stale_raw.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"no_safe_fix"}), encoding="utf-8")
    current_validated.write_text(json.dumps({"schema_version":"fix-dispatch-agent-result.v1", "task_id":"t1", "status":"patched", "patch":"diff --git a/docs/x b/docs/x\n"}), encoding="utf-8")

    with pytest.raises(Exception, match="duplicate fix result"):
        collect_agent_results(manifest, [stale_raw, current_validated])
