"""Memory text redaction helpers."""
from __future__ import annotations

from codex_review.security.redaction import redact_secrets


def redact_memory_text(text: str) -> str:
    return redact_secrets(text)
