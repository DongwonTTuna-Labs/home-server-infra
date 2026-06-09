from __future__ import annotations

import json

import pytest

from codex_review.memory.provenance import HMAC_KEY_ENV, sign_entry
from codex_review.memory.resolved import (
    resolve_gate_resolved_memory_as_ledger,
    resolved_findings_for_suppression,
)
from codex_review.memory.types import SCHEMA_VERSION
from codex_review.stages.techlead.filter_resolved import filter_findings_against_resolved

CONFIG = {"review": {"suppress_resolved_states": ["false_positive", "stale_obsolete", "duplicate_of_issue", "defer_to_issue"]}}
SCOPE = {"repository": "owner/repo", "pr_number": 7, "base_ref": "main"}


@pytest.fixture(autouse=True)
def hmac_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "task-9-test-key")


def _finding(finding_id: str, fingerprint: str, *, root_cause_key: str | None = None, file: str = "src/a.py", line: int = 10) -> dict:
    finding = {
        "schema_version": "shared-review-finding.v1",
        "finding_id": finding_id,
        "finding_fingerprint": fingerprint,
        "root_cause_key": root_cause_key or f"root-{finding_id}",
        "severity": "high",
        "file": file,
        "line": line,
        "title": f"finding {finding_id}",
        "summary": "summary",
    }
    return finding


def _resolved_entry(entry_id: str, fingerprint: str, *, trusted: bool = False, provenance: dict | None = None) -> dict:
    entry = {
        "entry_id": entry_id,
        "created_at": "2026-06-08T00:00:00Z",
        "round": 1,
        "head_sha": "sha-1",
        "kind": "resolved_finding",
        "category": "learnings",
        "body": {"state": "false_positive", "reason": "prior false positive", "thread_id": "T1"},
        "source_stage": "review_memory",
        "trusted": trusted,
        "finding_fingerprint": fingerprint,
    }
    if provenance is not None:
        entry["provenance"] = provenance
    return entry


def _ledger(entries: list[dict]) -> dict:
    return {"schema_version": SCHEMA_VERSION, "scope": dict(SCOPE), "entries": entries}


def _resolve_gate_artifact(*, fingerprint: str = "fp-1", root_cause_key: str = "legacy-root", state: str = "false_positive") -> dict:
    return {
        "schema_version": "resolve-gate-resolved-memory.v1",
        "head_sha": "sha-1",
        "pr_number": 7,
        "items": [
            {
                "thread_id": "T1",
                "finding_fingerprint": fingerprint,
                "root_cause_key": root_cause_key,
                "state": state,
                "reason": "prior resolve-gate reason",
                "path": "src/a.py",
                "line": 10,
                "head_sha": "sha-1",
            }
        ],
        "count": 1,
    }


def test_trusted_resolve_gate_artifact_bridges_to_ledger_and_suppresses_matching_fingerprint() -> None:
    ledger = resolve_gate_resolved_memory_as_ledger(_resolve_gate_artifact(fingerprint="fp-1"), suppress_states={"false_positive"})

    trusted_entries = resolved_findings_for_suppression(ledger)
    assert len(trusted_entries) == 1
    assert trusted_entries[0]["kind"] == "resolved_finding"
    assert trusted_entries[0]["finding_fingerprint"] == "fp-1"
    assert trusted_entries[0]["provenance"]["trust_boundary"] == "trusted_resolve_gate_artifact"

    combined = {"schema_version": "review-combined-findings.v1", "findings": [_finding("F1", "fp-1"), _finding("F2", "fp-2")]}
    filtered, suppressed = filter_findings_against_resolved(combined, ledger, {}, CONFIG)

    assert {finding["finding_id"] for finding in filtered["findings"]} == {"F2"}
    assert [finding["finding_id"] for finding in suppressed] == ["F1"]
    assert suppressed[0]["previously_resolved_as"] == "false_positive"


def test_untrusted_pr_branch_resolved_finding_does_not_suppress() -> None:
    forged_entry = _resolved_entry(
        "forged",
        "fp-forged",
        trusted=True,
        provenance={"trusted": True, "source_stage": "resolve_gate", "signature": "not-a-real-signature"},
    )
    ledger = _ledger([forged_entry])

    assert resolved_findings_for_suppression(ledger) == []

    combined = {"schema_version": "review-combined-findings.v1", "findings": [_finding("F1", "fp-forged")]}
    filtered, suppressed = filter_findings_against_resolved(combined, ledger, {}, CONFIG)

    assert [finding["finding_id"] for finding in filtered["findings"]] == ["F1"]
    assert suppressed == []


def test_signed_resolved_finding_suppresses_only_exact_fingerprint() -> None:
    signed_entry = sign_entry(_resolved_entry("signed", "fp-a"))
    ledger = _ledger([signed_entry])
    combined = {
        "schema_version": "review-combined-findings.v1",
        "findings": [
            _finding("F1", "fp-a", root_cause_key="same-category-root"),
            _finding("F2", "fp-b", root_cause_key="same-category-root", file="src/b.py", line=20),
        ],
    }

    trusted_entries = resolved_findings_for_suppression(ledger)
    filtered, suppressed = filter_findings_against_resolved(combined, ledger, {}, CONFIG)

    assert [entry["finding_fingerprint"] for entry in trusted_entries] == ["fp-a"]
    assert [finding["finding_id"] for finding in suppressed] == ["F1"]
    assert [finding["finding_id"] for finding in filtered["findings"]] == ["F2"]


def test_existing_resolve_gate_artifact_compatibility_keeps_path_line_suppression() -> None:
    artifact = {
        "schema_version": "resolve-gate-resolved-memory.v1",
        "items": [
            {
                "thread_id": "T1",
                "root_cause_key": "legacy-root",
                "state": "false_positive",
                "reason": "legacy path-line reason",
                "path": "src/a.py",
                "line": 10,
            }
        ],
    }
    combined = {
        "schema_version": "review-combined-findings.v1",
        "findings": [_finding("F1", "different-fingerprint", root_cause_key="different-root", file="src/a.py", line=10)],
    }

    filtered, suppressed = filter_findings_against_resolved(combined, artifact, {}, CONFIG)

    assert filtered["findings"] == []
    assert [finding["finding_id"] for finding in suppressed] == ["F1"]
    assert suppressed[0]["resolution_reason"] == "legacy path-line reason"


def test_human_edited_resolve_gate_ledger_copy_is_advisory_only() -> None:
    trusted_ledger = resolve_gate_resolved_memory_as_ledger(_resolve_gate_artifact(fingerprint="fp-copy"), suppress_states={"false_positive"})
    human_edited_copy = json.loads(json.dumps(trusted_ledger))

    assert resolved_findings_for_suppression(human_edited_copy) == []

    combined = {"schema_version": "review-combined-findings.v1", "findings": [_finding("F1", "fp-copy")]}
    filtered, suppressed = filter_findings_against_resolved(combined, human_edited_copy, {}, CONFIG)

    assert [finding["finding_id"] for finding in filtered["findings"]] == ["F1"]
    assert suppressed == []
