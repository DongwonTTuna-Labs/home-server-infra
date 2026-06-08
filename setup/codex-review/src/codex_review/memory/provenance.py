"""Trusted memory ledger entry provenance helpers."""
from __future__ import annotations

import copy
import hashlib
import hmac
import json
import os
from typing import Any

HMAC_KEY_ENV = "CODEX_MEMORY_HMAC_KEY"
PROVENANCE_SCHEMA_VERSION = "memory-provenance.v1"
HMAC_ALGORITHM = "hmac-sha256"
HMAC_KEY_ID = HMAC_KEY_ENV

_MUTABLE_PROVENANCE_FIELDS = frozenset({"provenance", "signature", "trusted"})


def canonicalize_entry(entry: dict[str, Any]) -> str:
    canonical = {key: value for key, value in entry.items() if key not in _MUTABLE_PROVENANCE_FIELDS}
    return json.dumps(canonical, sort_keys=True, separators=(",", ":"), default=str)


def _hmac_key() -> str | None:
    key = os.environ.get(HMAC_KEY_ENV)
    return key if key else None


def _signature_for(entry: dict[str, Any], key: str) -> str:
    material = canonicalize_entry(entry).encode("utf-8")
    return hmac.new(key.encode("utf-8"), material, hashlib.sha256).hexdigest()


def _provenance(*, trusted: bool, signature: str = "") -> dict[str, Any]:
    return {
        "schema_version": PROVENANCE_SCHEMA_VERSION,
        "trusted": trusted,
        "algorithm": HMAC_ALGORITHM,
        "key_id": HMAC_KEY_ID,
        "signature": signature,
    }


def sign_entry(entry: dict[str, Any]) -> dict[str, Any]:
    signed = copy.deepcopy(entry)
    key = _hmac_key()
    if not key:
        signed["trusted"] = False
        signed["provenance"] = _provenance(trusted=False)
        return signed

    signature = _signature_for(signed, key)
    signed["trusted"] = True
    signed["provenance"] = _provenance(trusted=True, signature=signature)
    return signed


def _verify_entry_with_metadata(entry: dict[str, Any]) -> dict[str, Any]:
    verified = copy.deepcopy(entry)
    provenance = verified.get("provenance") if isinstance(verified.get("provenance"), dict) else {}
    signature = str(provenance.get("signature") or "")
    key = _hmac_key()

    valid = False
    if (
        key
        and signature
        and provenance.get("algorithm") == HMAC_ALGORITHM
        and provenance.get("key_id") == HMAC_KEY_ID
    ):
        expected = _signature_for(verified, key)
        valid = hmac.compare_digest(signature, expected)

    next_provenance = dict(provenance)
    next_provenance.setdefault("schema_version", PROVENANCE_SCHEMA_VERSION)
    next_provenance.setdefault("algorithm", HMAC_ALGORITHM)
    next_provenance.setdefault("key_id", HMAC_KEY_ID)
    next_provenance.setdefault("signature", signature)
    next_provenance["trusted"] = valid
    verified["trusted"] = valid
    verified["provenance"] = next_provenance
    return verified


def verified_entry(entry: dict[str, Any]) -> dict[str, Any]:
    return _verify_entry_with_metadata(entry)


def verify_entry(entry: dict[str, Any]) -> bool:
    return bool(_verify_entry_with_metadata(entry)["trusted"])


def is_trusted_for_suppression(entry: dict[str, Any]) -> bool:
    finding_fingerprint = str(entry.get("finding_fingerprint") or "").strip()
    return bool(finding_fingerprint) and verify_entry(entry)
