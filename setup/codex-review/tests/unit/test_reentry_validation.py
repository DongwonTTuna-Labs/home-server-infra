import pytest
from codex_review.stages.reentry.validate import validate_reentry_record


def test_reentry_validate_blocks_pushed_without_commit():
    with pytest.raises(Exception):
        validate_reentry_record({"schema_version":"reentry-loop-state.v1", "pushed": True, "next_entry":"resolve_gate_on_synchronize"})


def test_reentry_validate_no_push():
    out = validate_reentry_record({"schema_version":"reentry-loop-state.v1", "pushed": False, "next_entry":"none"})
    assert out["next_entry"] == "none"
