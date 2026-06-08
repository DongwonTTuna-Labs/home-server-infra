"""Conservative secret detection and redaction."""
from __future__ import annotations

from collections import Counter
import math
import re
from typing import Any

from codex_review.core.errors import PolicyViolation

REDACTION_MARKER = "[REDACTED_SECRET]"
HIGH_ENTROPY_PATTERN_NAME = "high_entropy_token"
HIGH_ENTROPY_MIN_LENGTH = 40
HIGH_ENTROPY_MIN_UNIQUE_CHARS = 16
HIGH_ENTROPY_MIN_BITS_PER_CHAR = 4.2

SECRET_PATTERNS = [
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?im)\b(api[_-]?key|secret|token|password|credential|private[_-]?key)\b[ \t]*[:=][ \t]*(?:\|[ \t]*)?\n(?:[ \t]+[A-Za-z0-9_+/\-.=]{12,}[ \t]*\n?){2,}"),
    re.compile(r"(?i)\b(api[_-]?key|secret|token|password|credential)\b[ \t]*[:=][ \t]*['\"]?([A-Za-z0-9_+/\-.=]{20,})"),
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-(?:proj-)?[A-Za-z0-9][A-Za-z0-9_-]{20,}"),
    re.compile(r"xox[baprs]-[A-Za-z0-9-]{20,}"),
    re.compile(r"AKIA[0-9A-Z]{16}"),
]

HIGH_ENTROPY_CANDIDATE_RE = re.compile(r"(?<![A-Za-z0-9_+/=.-])([A-Za-z0-9_+/=.-]{40,})(?![A-Za-z0-9_+/=.-])")
PATH_LIKE_EXTENSION_RE = re.compile(r"\.[A-Za-z0-9]{1,12}$")


def _looks_like_filesystem_path(candidate: str) -> bool:
    if "/" not in candidate and "\\" not in candidate:
        return False
    normalized = candidate.replace("\\", "/")
    segments = [segment for segment in normalized.split("/") if segment]
    if not segments:
        return False
    if any(segment in {".", ".."} for segment in segments):
        return True
    if normalized.startswith(("/", "./", "../", "~/")) and len(segments) >= 2:
        return True
    if len(segments) < 2:
        return False
    if PATH_LIKE_EXTENSION_RE.search(segments[-1]):
        return True
    return any("-" in segment or "." in segment for segment in segments)


def _shannon_entropy(candidate: str) -> float:
    if not candidate:
        return 0.0
    length = len(candidate)
    counts = Counter(candidate)
    return -sum((count / length) * math.log2(count / length) for count in counts.values())


def _looks_like_high_entropy_secret(candidate: str) -> bool:
    if _looks_like_filesystem_path(candidate):
        return False
    if len(candidate) < HIGH_ENTROPY_MIN_LENGTH:
        return False
    if len(set(candidate)) < HIGH_ENTROPY_MIN_UNIQUE_CHARS:
        return False
    has_lower = any(char.islower() for char in candidate)
    has_upper = any(char.isupper() for char in candidate)
    has_digit = any(char.isdigit() for char in candidate)
    has_base64_symbol = any(char in "+=" for char in candidate)
    if not ((has_lower and has_upper and has_digit) or has_base64_symbol):
        return False
    return _shannon_entropy(candidate) >= HIGH_ENTROPY_MIN_BITS_PER_CHAR


def _contains_high_entropy_secret(text: str) -> bool:
    return any(_looks_like_high_entropy_secret(match.group(1)) for match in HIGH_ENTROPY_CANDIDATE_RE.finditer(text or ""))


def _redact_high_entropy_secrets(text: str) -> str:
    def replace(match: re.Match[str]) -> str:
        candidate = match.group(1)
        return REDACTION_MARKER if _looks_like_high_entropy_secret(candidate) else candidate

    return HIGH_ENTROPY_CANDIDATE_RE.sub(replace, text)


def redact_secrets(text: str) -> str:
    redacted = text or ""
    for pattern in SECRET_PATTERNS:
        redacted = pattern.sub(REDACTION_MARKER, redacted)
    return _redact_high_entropy_secrets(redacted)


def assert_no_secret_patterns(text: str, context: str = "text") -> None:
    for pattern in SECRET_PATTERNS:
        if pattern.search(text or ""):
            raise PolicyViolation(f"secret-like material detected in {context}")
    if _contains_high_entropy_secret(text or ""):
        raise PolicyViolation(f"secret-like material detected in {context}")


def scan_patch_for_secrets(patch_text: str) -> list[dict[str, Any]]:
    findings=[]
    for line_no, line in enumerate((patch_text or "").splitlines(), 1):
        if line.startswith("+") and not line.startswith("+++"):
            for pattern in SECRET_PATTERNS:
                if pattern.search(line):
                    findings.append({"line": line_no, "pattern": pattern.pattern})
            if _contains_high_entropy_secret(line):
                findings.append({"line": line_no, "pattern": HIGH_ENTROPY_PATTERN_NAME})
    return findings


def safe_log_value(value: Any) -> str:
    text = redact_secrets(str(value))
    return text if len(text) <= 1000 else text[:1000] + "...[truncated]"
