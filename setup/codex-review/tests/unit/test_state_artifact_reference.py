"""Tests for explicit loop state artifact references."""
from __future__ import annotations

import pytest

from codex_review.core.errors import ValidationError
from codex_review.loop.state import (
    build_state_artifact_download_command,
    deterministic_state_artifact_name,
    state_artifact_pointer_payload,
    validate_state_artifact_reference,
)


def test_state_artifact_pointer_payload_carries_run_id_and_artifact_name():
    pointer = state_artifact_pointer_payload(
        state_run_id=123456789,
        state_artifact_name="codex-loop-state-corr-7.json",
    )

    assert pointer == {
        "state_run_id": "123456789",
        "state_artifact_name": "codex-loop-state-corr-7.json",
    }


def test_state_artifact_reference_rejects_missing_state_pointer():
    with pytest.raises(ValidationError, match="state_run_id is required"):
        validate_state_artifact_reference({"state_artifact_name": "codex-loop-state.json"})

    with pytest.raises(ValidationError, match="state_artifact_name is required"):
        validate_state_artifact_reference({"state_run_id": "123"})


def test_state_artifact_reference_allows_only_initial_empty_when_explicitly_requested():
    assert validate_state_artifact_reference(
        {"iteration": 0},
        allow_initial_empty=True,
    ) == {"state_run_id": "", "state_artifact_name": ""}

    with pytest.raises(ValidationError, match="state_run_id is required"):
        validate_state_artifact_reference({"iteration": 1}, allow_initial_empty=True)


def test_state_artifact_reference_rejects_path_names_and_non_run_ids():
    with pytest.raises(ValidationError, match="positive integer"):
        validate_state_artifact_reference({"state_run_id": "abc", "state_artifact_name": "state.json"})

    with pytest.raises(ValidationError, match="file name"):
        validate_state_artifact_reference({"state_run_id": "123", "state_artifact_name": "nested/state.json"})


def test_download_command_uses_explicit_run_id_and_artifact_name_without_head_sha():
    command = build_state_artifact_download_command(
        {"state_run_id": "987654321", "state_artifact_name": "codex-loop-state-corr-6.json"},
        output_dir="prior-state",
    )

    assert command == [
        "gh",
        "run",
        "download",
        "987654321",
        "--name",
        "codex-loop-state-corr-6.json",
        "--dir",
        "prior-state",
    ]
    assert "--headSha" not in command
    assert "headSha" not in " ".join(command)


def test_duplicate_same_head_reruns_are_disambiguated_by_run_id_not_artifact_name():
    first = build_state_artifact_download_command(
        {"state_run_id": "111", "state_artifact_name": "codex-loop-state-same-head-2.json"},
        output_dir="state",
    )
    rerun = build_state_artifact_download_command(
        {"state_run_id": "222", "state_artifact_name": "codex-loop-state-same-head-2.json"},
        output_dir="state",
    )

    assert first[3] == "111"
    assert rerun[3] == "222"
    assert first[5] == rerun[5]


def test_deterministic_state_artifact_name_is_machine_readable():
    assert deterministic_state_artifact_name("corr/id 42", 3) == "codex-loop-state-corr-id-42-3.json"
