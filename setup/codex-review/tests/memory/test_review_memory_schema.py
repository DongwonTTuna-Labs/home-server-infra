from __future__ import annotations

from dataclasses import FrozenInstanceError

import pytest

from codex_review.core.errors import ValidationError
from codex_review.core.schema import load_schema_json, validate_json_schema
from codex_review.memory.types import (
    CATEGORY_NOTEPAD_FILES,
    SCHEMA_VERSION,
    is_entry_valid,
    make_review_memory_entry,
    make_review_memory_ledger,
    validate_review_memory_entry,
    validate_review_memory_ledger,
)


def _entry_dict() -> dict:
    return make_review_memory_entry(
        entry_id="entry-1",
        created_at="2026-06-08T00:00:00Z",
        round=1,
        head_sha="abc123",
        kind="learning",
        category="learnings",
        body={"summary": "Schema entries are knowledge only."},
        source_stage="review",
        trusted=True,
        finding_fingerprint="fingerprint-1",
        provenance={"artifact": "review.json"},
    ).to_dict()


def _ledger_dict() -> dict:
    ledger = make_review_memory_ledger(
        scope={"repository": "owner/repo", "pr_number": 123, "base_ref": "main"},
        entries=(make_review_memory_entry(**_entry_dict()),),
    )
    return ledger.to_dict()


def test_review_memory_schema_is_strict_and_knowledge_only():
    schema = load_schema_json(SCHEMA_VERSION)

    assert schema["additionalProperties"] is False
    assert schema["properties"]["scope"]["additionalProperties"] is False
    entry_schema = schema["properties"]["entries"]["items"]
    assert entry_schema["additionalProperties"] is False
    assert "orchestration_route" not in entry_schema["properties"]["kind"]["enum"]
    assert set(entry_schema["properties"]["kind"]["enum"]) == {
        "fix_applied",
        "decision",
        "learning",
        "rejected_approach",
        "open_risk",
        "resolved_finding",
    }
    assert set(entry_schema["properties"]["category"]["enum"]) == set(CATEGORY_NOTEPAD_FILES)


def test_valid_review_memory_ledger_and_entry_are_accepted():
    entry = _entry_dict()
    ledger = _ledger_dict()

    validate_review_memory_entry(entry)
    validate_review_memory_ledger(ledger)
    validate_json_schema(ledger, SCHEMA_VERSION)


def test_missing_required_entry_field_is_rejected():
    entry = _entry_dict()
    entry.pop("head_sha")

    with pytest.raises(ValidationError):
        validate_review_memory_entry(entry)


def test_unknown_kind_is_rejected():
    entry = _entry_dict()
    entry["kind"] = "orchestration_route"

    with pytest.raises(ValidationError):
        validate_review_memory_entry(entry)


def test_is_entry_valid_returns_true_for_valid_entry():
    assert is_entry_valid(_entry_dict()) is True


def test_is_entry_valid_returns_false_for_missing_required_field():
    entry = _entry_dict()
    entry.pop("head_sha")

    assert is_entry_valid(entry) is False


def test_is_entry_valid_returns_false_for_unknown_kind():
    entry = _entry_dict()
    entry["kind"] = "orchestration_route"

    assert is_entry_valid(entry) is False


def test_is_entry_valid_returns_false_for_unknown_category():
    entry = _entry_dict()
    entry["category"] = "routes"

    assert is_entry_valid(entry) is False


def test_is_entry_valid_returns_false_for_malformed_body():
    entry = _entry_dict()
    entry["body"] = "not an object"

    assert is_entry_valid(entry) is False


def test_unknown_ledger_property_is_rejected_by_schema():
    ledger = _ledger_dict()
    ledger["orchestration_route"] = "fix_dispatch"

    with pytest.raises(Exception):
        validate_json_schema(ledger, SCHEMA_VERSION)


def test_review_memory_entry_dataclass_is_frozen():
    entry = make_review_memory_entry(
        entry_id="entry-immutable",
        created_at="2026-06-08T00:00:00Z",
        round=1,
        head_sha="abc123",
        kind="decision",
        category="decisions",
        body={"summary": "Keep routing out of review memory."},
        source_stage="techlead",
        trusted=True,
    )

    with pytest.raises(FrozenInstanceError):
        entry.kind = "learning"
