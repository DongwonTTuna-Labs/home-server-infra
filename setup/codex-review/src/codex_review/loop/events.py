"""Audit event artifacts."""
from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from codex_review.security.redaction import redact_secrets


def record_event(event_type: str, source: str, evidence: dict[str, Any] | None = None) -> dict[str, Any]:
    return {"event_type": event_type, "source": source, "evidence": evidence or {}, "created_at": datetime.now(timezone.utc).isoformat()}


def append_event_log(path: str | Path, event: dict[str, Any]) -> Path:
    p=Path(path); p.parent.mkdir(parents=True, exist_ok=True)
    with p.open("a", encoding="utf-8") as fh:
        fh.write(json.dumps(event, ensure_ascii=False, sort_keys=True)+"\n")
    return p


def events_for_stage(stage: str, artifacts: dict[str, Any]) -> list[dict[str, Any]]:
    return [record_event("ARTIFACT", stage, {"name": k, "value": str(v)[:200]}) for k, v in (artifacts or {}).items()]


def render_event_summary(events: list[dict[str, Any]]) -> str:
    lines=["## Codex Review events"]
    for e in events:
        lines.append(f"- `{e.get('event_type')}` from `{e.get('source')}` at {e.get('created_at')}: {redact_secrets(str(e.get('evidence')))}")
    return "\n".join(lines)
