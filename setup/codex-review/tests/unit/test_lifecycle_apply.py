import pytest
from codex_review.stages.resolve_gate.validate import validate_lifecycle_result
from codex_review.stages.resolve_gate.apply import apply_lifecycle_result


def inventory():
    return {"schema_version":"resolve-gate-thread-inventory.v1","items":[{"thread_id":"T1","forced_needs_human":False,"is_current_head":False,"root_cause_key":"rc1"}]}


def test_lifecycle_validation_requires_exact_thread_coverage():
    with pytest.raises(Exception):
        validate_lifecycle_result({"decisions":[]}, inventory())


def test_terminal_lifecycle_requires_evidence():
    with pytest.raises(Exception):
        validate_lifecycle_result({"decisions":[{"thread_id":"T1","state":"resolved_by_code"}]}, inventory())


def test_apply_lifecycle_dry_run_builds_idempotent_plan():
    result=validate_lifecycle_result({"decisions":[{"thread_id":"T1","state":"resolved_by_code","evidence":"fixed"}]}, inventory())
    report=apply_lifecycle_result(result, {"repository":"o/r","pr_number":1}, None, {}, dry_run=True)
    assert report["dry_run"] is True
    assert report["resolved_thread_ids"] == ["T1"]
