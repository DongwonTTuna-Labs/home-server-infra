"""Conservative secret detection and redaction."""
from __future__ import annotations

import re
from typing import Any

from codex_review.core.errors import PolicyViolation

SECRET_PATTERNS = [
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?i)(api[_-]?key|secret|token|password)\s*[:=]\s*['\"]?([A-Za-z0-9_\-]{20,})"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"xox[baprs]-[A-Za-z0-9-]{20,}"),
    re.compile(r"AKIA[0-9A-Z]{16}"),
]


def redact_secrets(text: str) -> str:
    redacted = text or ""
    for pattern in SECRET_PATTERNS:
        redacted = pattern.sub("[REDACTED_SECRET]", redacted)
    return redacted


def assert_no_secret_patterns(text: str, context: str = "text") -> None:
    for pattern in SECRET_PATTERNS:
        if pattern.search(text or ""):
            raise PolicyViolation(f"secret-like material detected in {context}")


def scan_patch_for_secrets(patch_text: str) -> list[dict[str, Any]]:
    findings=[]
    for line_no, line in enumerate((patch_text or "").splitlines(), 1):
        if line.startswith("+") and not line.startswith("+++"):
            for pattern in SECRET_PATTERNS:
                if pattern.search(line):
                    findings.append({"line": line_no, "pattern": pattern.pattern})
    return findings


def safe_log_value(value: Any) -> str:
    text = redact_secrets(str(value))
    return text if len(text) <= 1000 else text[:1000] + "...[truncated]"
