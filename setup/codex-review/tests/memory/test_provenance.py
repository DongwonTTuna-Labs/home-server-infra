"""Tests for trusted memory ledger entry provenance."""
from __future__ import annotations

import copy

import pytest

from codex_review.memory.provenance import (
    HMAC_ALGORITHM,
    HMAC_KEY_ENV,
    HMAC_KEY_ID,
    PROVENANCE_SCHEMA_VERSION,
    canonicalize_entry,
    is_trusted_for_suppression,
    sign_entry,
    verified_entry,
    verify_entry,
)


def _entry(**overrides):
    entry = {
        "schema_version": "codex-memory-ledger-entry.v1",
        "kind": "resolved_finding",
        "head_sha": "a" * 40,
        "finding_fingerprint": "review:security:abc123",
        "body": {"finding_id": "security-1", "status": "resolved"},
    }
    entry.update(overrides)
    return entry


@pytest.fixture(autouse=True)
def hmac_key(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "testkey")


def test_sign_entry_returns_copy_with_trusted_provenance() -> None:
    entry = _entry(body={"finding_id": "security-1", "notes": ["fixed"]})
    original = copy.deepcopy(entry)

    signed = sign_entry(entry)

    assert entry == original
    assert signed is not entry
    assert signed["trusted"] is True
    assert signed["provenance"] == {
        "schema_version": PROVENANCE_SCHEMA_VERSION,
        "trusted": True,
        "algorithm": HMAC_ALGORITHM,
        "key_id": HMAC_KEY_ID,
        "signature": signed["provenance"]["signature"],
    }
    assert len(signed["provenance"]["signature"]) == 64
    assert verify_entry(signed) is True


def test_verified_entry_returns_normalized_metadata_copy() -> None:
    signed = sign_entry(_entry())
    original = copy.deepcopy(signed)
    untrusted_claim = {**signed, "trusted": False, "provenance": {**signed["provenance"], "trusted": False}}

    verified = verified_entry(untrusted_claim)

    assert signed == original
    assert verified is not untrusted_claim
    assert verified["trusted"] is True
    assert verified["provenance"]["trusted"] is True
    assert verify_entry(untrusted_claim) is True


def test_canonicalization_excludes_mutable_provenance_fields() -> None:
    signed = sign_entry(_entry())
    edited_provenance = {
        **signed,
        "trusted": False,
        "signature": "human-edit",
        "provenance": {**signed["provenance"], "trusted": False},
    }

    assert canonicalize_entry(signed) == canonicalize_entry(edited_provenance)
    assert verify_entry(edited_provenance) is True


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("body", {"finding_id": "security-1", "status": "regressed"}),
        ("head_sha", "b" * 40),
        ("kind", "human_note"),
    ],
)
def test_tampering_with_signed_entry_body_fails_verification(field: str, value: object) -> None:
    signed = sign_entry(_entry())
    tampered = {**signed, field: value}

    assert verify_entry(tampered) is False
    assert verified_entry(tampered)["trusted"] is False
    assert is_trusted_for_suppression(tampered) is False


def test_missing_signature_is_not_trusted() -> None:
    unsigned = _entry(provenance={"trusted": True, "algorithm": HMAC_ALGORITHM, "key_id": HMAC_KEY_ID})

    assert verify_entry(unsigned) is False
    assert verified_entry(unsigned)["trusted"] is False
    assert is_trusted_for_suppression(unsigned) is False


def test_missing_key_degrades_to_untrusted(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv(HMAC_KEY_ENV, raising=False)

    signed = sign_entry(_entry())

    assert signed["trusted"] is False
    assert signed["provenance"]["trusted"] is False
    assert signed["provenance"]["signature"] == ""
    assert verify_entry(signed) is False
    assert verified_entry(signed)["trusted"] is False
    assert is_trusted_for_suppression(signed) is False


def test_is_trusted_for_suppression_requires_finding_fingerprint() -> None:
    signed_without_fingerprint = sign_entry(_entry(finding_fingerprint=""))

    assert verify_entry(signed_without_fingerprint) is True
    assert is_trusted_for_suppression(signed_without_fingerprint) is False


def test_unsigned_human_edited_entry_cannot_claim_trust() -> None:
    human_entry = _entry(trusted=True, provenance={"trusted": True, "signature": "not-a-real-signature"})

    assert verify_entry(human_entry) is False
    assert verified_entry(human_entry)["trusted"] is False
    assert is_trusted_for_suppression(human_entry) is False
