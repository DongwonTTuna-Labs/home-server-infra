from __future__ import annotations

import json
from copy import deepcopy
from pathlib import Path

import pytest

from codex_review.core.errors import ValidationError
from codex_review.memory.ledger import append_entries, read_ledger, read_ledger_file, write_ledger
from codex_review.memory.types import SCHEMA_VERSION, validate_review_memory_ledger

SCOPE = {"repository": "owner/repo", "pr_number": 7, "base_ref": "main"}


def _empty_ledger() -> dict:
    return {"schema_version": SCHEMA_VERSION, "scope": dict(SCOPE), "entries": []}


def _entry(entry_id: str, *, created_at: str = "2026-06-08T00:00:00Z", round: int = 1) -> dict:
    return {
        "entry_id": entry_id,
        "created_at": created_at,
        "round": round,
        "head_sha": f"sha-{entry_id}",
        "kind": "learning",
        "category": "learnings",
        "body": {"summary": f"entry {entry_id}"},
        "source_stage": "review",
        "trusted": True,
    }


def _write_json(path: Path, payload: object) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload), encoding="utf-8")
    return path


def test_read_ledger_absent_bootstraps_empty_review_memory_ledger(tmp_path: Path) -> None:
    ledger = read_ledger(tmp_path, SCOPE)

    assert ledger == _empty_ledger()
    validate_review_memory_ledger(ledger)


def test_append_entries_preserves_prior_entries_and_does_not_mutate_inputs() -> None:
    existing_entry = _entry("existing")
    new_entry = _entry("new", created_at="2026-06-08T00:01:00Z", round=2)
    ledger = {"schema_version": SCHEMA_VERSION, "scope": dict(SCOPE), "entries": [existing_entry]}
    original_ledger = deepcopy(ledger)
    original_new_entry = deepcopy(new_entry)

    updated = append_entries(ledger, [new_entry])

    assert ledger == original_ledger
    assert new_entry == original_new_entry
    assert updated["entries"] == [existing_entry, new_entry]
    updated["entries"][1]["body"]["summary"] = "mutated copy"
    assert new_entry["body"]["summary"] == "entry new"


def test_append_entries_ignores_duplicate_new_entry_ids() -> None:
    existing_entry = _entry("existing")
    first_new = _entry("new", created_at="2026-06-08T00:01:00Z", round=2)
    duplicate_existing = _entry("existing", created_at="2026-06-08T00:02:00Z", round=3)
    duplicate_new = _entry("new", created_at="2026-06-08T00:03:00Z", round=4)
    duplicate_new["body"] = {"summary": "second duplicate should be ignored"}

    updated = append_entries(
        {"schema_version": SCHEMA_VERSION, "scope": dict(SCOPE), "entries": [existing_entry]},
        [duplicate_existing, first_new, duplicate_new],
    )

    assert [entry["entry_id"] for entry in updated["entries"]] == ["existing", "new"]
    assert updated["entries"][1]["body"]["summary"] == "entry new"


def test_read_ledger_file_corrupt_json_fails_safe_with_warning(tmp_path: Path) -> None:
    path = tmp_path / "ledger.json"
    path.write_text("not json", encoding="utf-8")

    ledger = read_ledger_file(path, SCOPE)

    assert ledger["entries"] == []
    assert ledger["schema_version"] == SCHEMA_VERSION
    assert ledger["scope"] == SCOPE
    assert ledger["warnings"][0]["code"] == "malformed_json"


@pytest.mark.parametrize(
    ("payload", "warning_code"),
    [
        (["not", "an", "object"], "non_object"),
        ({"schema_version": "other.v1", "scope": SCOPE, "entries": []}, "invalid_ledger"),
        (
            {
                "schema_version": SCHEMA_VERSION,
                "scope": SCOPE,
                "entries": [{**_entry("invalid"), "body": "not an object"}],
            },
            "invalid_ledger",
        ),
    ],
)
def test_read_ledger_file_invalid_payloads_fail_safe_to_empty_entries(
    tmp_path: Path,
    payload: object,
    warning_code: str,
) -> None:
    path = _write_json(tmp_path / "ledger.json", payload)

    ledger = read_ledger_file(path, SCOPE)

    assert ledger["entries"] == []
    assert ledger["warnings"][0]["code"] == warning_code


def test_write_ledger_validates_stamps_and_round_trips(tmp_path: Path) -> None:
    ledger = append_entries(
        _empty_ledger(),
        [
            _entry("one"),
            _entry("two", created_at="2026-06-08T00:01:00Z", round=2),
        ],
    )
    out_path = tmp_path / "nested" / "ledger.json"

    written = write_ledger(out_path, {"scope": dict(SCOPE), "entries": ledger["entries"]})

    assert written == out_path
    raw = out_path.read_text(encoding="utf-8")
    assert raw.endswith("\n")
    assert json.loads(raw)["schema_version"] == SCHEMA_VERSION
    assert read_ledger_file(out_path, SCOPE) == ledger


def test_write_ledger_rejects_non_schema_warning_fields(tmp_path: Path) -> None:
    invalid = {**_empty_ledger(), "warnings": [{"code": "invalid_ledger"}]}

    with pytest.raises(ValidationError):
        write_ledger(tmp_path / "ledger.json", invalid)

    assert not (tmp_path / "ledger.json").exists()


def test_append_entries_caps_by_dropping_oldest_entries_first() -> None:
    ledger = append_entries(
        _empty_ledger(),
        [
            _entry("one", created_at="2026-06-08T00:00:00Z", round=1),
            _entry("two", created_at="2026-06-08T00:01:00Z", round=2),
        ],
    )

    updated = append_entries(
        ledger,
        [
            _entry("three", created_at="2026-06-08T00:02:00Z", round=3),
            _entry("four", created_at="2026-06-08T00:03:00Z", round=4),
        ],
        max_entries=3,
    )

    assert [entry["entry_id"] for entry in updated["entries"]] == ["two", "three", "four"]
    validate_review_memory_ledger(updated)
