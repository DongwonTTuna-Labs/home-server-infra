"""Codex marker rendering and parsing."""
from __future__ import annotations

import hashlib
import json
import re
from typing import Any

MARKER_RE = re.compile(r"<!--\s*(?P<name>codex-review:[\w:-]+)(?:\s+(?P<payload>\{.*?\}))?\s*-->", re.DOTALL)


def render_marker(name: str, payload: dict[str, Any] | None = None) -> str:
    data = "" if payload is None else " " + json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return f"<!-- {name}{data} -->"


def parse_marker(text: str, name: str) -> dict[str, Any] | None:
    for match in MARKER_RE.finditer(text or ""):
        if match.group("name") == name:
            payload = match.group("payload")
            if not payload:
                return {}
            try:
                data=json.loads(payload)
            except json.JSONDecodeError:
                return {"_invalid": payload}
            return data if isinstance(data, dict) else {"value": data}
    return None


def has_marker(text: str, name: str) -> bool:
    return parse_marker(text, name) is not None or (name in (text or ""))


def render_inline_review_marker(finding_id: str, root_cause_key: str) -> str:
    return render_marker("codex-review:inline", {"finding_id": finding_id, "root_cause_key": root_cause_key})


def render_lifecycle_marker(thread_id: str, state: str, evidence_hash: str) -> str:
    return render_marker("codex-review:lifecycle", {"thread_id": thread_id, "state": state, "evidence_hash": evidence_hash})


def extract_root_cause_metadata(text: str) -> dict[str, Any]:
    for marker in ["codex-review:inline", "codex-review:lifecycle", "codex-review:deferred-issue"]:
        payload=parse_marker(text or "", marker)
        if payload is not None:
            if payload.get("_invalid"):
                return {"root_cause_key": None, "marker": marker, "payload": payload, "valid": False}
            root = payload.get("root_cause_key") or payload.get("root_cause") or payload.get("key")
            if root:
                return {"root_cause_key": str(root), "marker": marker, "payload": payload, "valid": True}
            if marker == "codex-review:lifecycle" and payload.get("thread_id"):
                return {"root_cause_key": str(payload["thread_id"]), "marker": marker, "payload": payload, "valid": True}
            return {"root_cause_key": None, "marker": marker, "payload": payload, "valid": False}
    # Fallback: root_cause_key: abc in body.
    m=re.search(r"root[_ -]?cause[_ -]?key\s*[:=]\s*([A-Za-z0-9_.:-]+)", text or "", re.I)
    if m:
        return {"root_cause_key": m.group(1), "marker": "text", "payload": {}, "valid": True}
    digest = hashlib.sha256((text or "").encode("utf-8")).hexdigest()[:16] if text else None
    return {"root_cause_key": digest, "marker": None, "payload": {}, "valid": bool(digest)}
