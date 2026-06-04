"""Typed errors used by the Codex Review workflow helpers."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping


class CodexReviewError(RuntimeError):
    """Base class for workflow helper failures."""


class ValidationError(CodexReviewError):
    """Raised when an artifact or model output violates a contract."""


class PolicyViolation(CodexReviewError):
    """Raised when a security or workflow policy check fails."""


class GitHubError(CodexReviewError):
    """Raised for GitHub API failures."""


@dataclass(frozen=True)
class ErrorReport:
    type: str
    message: str
    context: Mapping[str, Any]

    def as_dict(self) -> dict[str, Any]:
        return {"type": self.type, "message": self.message, "context": dict(self.context)}


def format_error(error: BaseException, context: Mapping[str, Any] | None = None) -> dict[str, Any]:
    return ErrorReport(type=error.__class__.__name__, message=str(error), context=context or {}).as_dict()


def raise_policy_violation(reason: str, evidence: Any | None = None) -> None:
    suffix = f" evidence={evidence!r}" if evidence is not None else ""
    raise PolicyViolation(f"{reason}{suffix}")


def raise_validation_error(reason: str, artifact: Any | None = None) -> None:
    suffix = f" artifact={artifact!r}" if artifact is not None else ""
    raise ValidationError(f"{reason}{suffix}")
