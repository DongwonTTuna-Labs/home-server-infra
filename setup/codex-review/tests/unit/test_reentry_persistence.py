import pytest

from codex_review.stages.reentry.validate import validate_reentry_record


def test_pushed_reentry_requires_persistence():
    with pytest.raises(Exception):
        validate_reentry_record({"schema_version":"reentry-loop-state.v1", "pushed": True, "commit_sha":"abc", "next_entry":"resolve_gate_on_synchronize", "persisted": False})


def test_pushed_reentry_allows_persisted_record():
    record = validate_reentry_record({"schema_version":"reentry-loop-state.v1", "pushed": True, "commit_sha":"abc", "next_entry":"resolve_gate_on_synchronize", "persisted": True})
    assert record["persisted"] is True
