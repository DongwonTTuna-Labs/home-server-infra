"""Typed review-memory.v1 ledger entries."""
from __future__ import annotations

from dataclasses import dataclass, field
from types import MappingProxyType
from typing import Any, Literal, Mapping, TypedDict

from codex_review.core.errors import ValidationError
from codex_review.core.schema import require_schema_version, validate_enum, validate_json_schema, validate_required_keys

SCHEMA_VERSION = "review-memory.v1"

ReviewMemoryKind = Literal[
    "fix_applied",
    "decision",
    "learning",
    "rejected_approach",
    "open_risk",
    "resolved_finding",
]
ReviewMemoryCategory = Literal["learnings", "decisions", "issues", "problems"]

REVIEW_MEMORY_KINDS: tuple[str, ...] = (
    "fix_applied",
    "decision",
    "learning",
    "rejected_approach",
    "open_risk",
    "resolved_finding",
)
REVIEW_MEMORY_CATEGORIES: tuple[str, ...] = ("learnings", "decisions", "issues", "problems")
CATEGORY_NOTEPAD_FILES: Mapping[str, str] = MappingProxyType(
    {
        "learnings": "learnings.md",
        "decisions": "decisions.md",
        "issues": "issues.md",
        "problems": "problems.md",
    }
)

ENTRY_REQUIRED_KEYS: tuple[str, ...] = (
    "entry_id",
    "created_at",
    "round",
    "head_sha",
    "kind",
    "category",
    "body",
    "source_stage",
    "trusted",
)
LEDGER_REQUIRED_KEYS: tuple[str, ...] = ("schema_version", "scope", "entries")
SCOPE_REQUIRED_KEYS: tuple[str, ...] = ("repository", "pr_number", "base_ref")


class ReviewMemoryScope(TypedDict):
    repository: str
    pr_number: int
    base_ref: str


class ReviewMemoryEntryPayload(TypedDict, total=False):
    entry_id: str
    created_at: str
    round: int
    head_sha: str
    kind: ReviewMemoryKind
    category: ReviewMemoryCategory
    body: dict[str, Any]
    source_stage: str
    trusted: bool
    finding_fingerprint: str
    provenance: dict[str, Any]


@dataclass(frozen=True, slots=True)
class ReviewMemoryEntry:
    entry_id: str
    created_at: str
    round: int
    head_sha: str
    kind: ReviewMemoryKind
    category: ReviewMemoryCategory
    body: Mapping[str, Any] = field(repr=False)
    source_stage: str
    trusted: bool
    finding_fingerprint: str | None = None
    provenance: Mapping[str, Any] | None = field(default=None, repr=False)

    def __post_init__(self) -> None:
        validate_review_memory_entry(self.to_dict())

    def to_dict(self) -> dict[str, Any]:
        payload: dict[str, Any] = {
            "entry_id": self.entry_id,
            "created_at": self.created_at,
            "round": self.round,
            "head_sha": self.head_sha,
            "kind": self.kind,
            "category": self.category,
            "body": dict(self.body),
            "source_stage": self.source_stage,
            "trusted": self.trusted,
        }
        if self.finding_fingerprint is not None:
            payload["finding_fingerprint"] = self.finding_fingerprint
        if self.provenance is not None:
            payload["provenance"] = dict(self.provenance)
        return payload


@dataclass(frozen=True, slots=True)
class ReviewMemoryLedger:
    scope: ReviewMemoryScope
    entries: tuple[ReviewMemoryEntry, ...]
    schema_version: str = SCHEMA_VERSION

    def to_dict(self) -> dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "scope": dict(self.scope),
            "entries": [entry.to_dict() for entry in self.entries],
        }


def make_review_memory_entry(
    *,
    entry_id: str,
    created_at: str,
    round: int,
    head_sha: str,
    kind: ReviewMemoryKind,
    category: ReviewMemoryCategory,
    body: Mapping[str, Any],
    source_stage: str,
    trusted: bool,
    finding_fingerprint: str | None = None,
    provenance: Mapping[str, Any] | None = None,
) -> ReviewMemoryEntry:
    return ReviewMemoryEntry(
        entry_id=entry_id,
        created_at=created_at,
        round=round,
        head_sha=head_sha,
        kind=kind,
        category=category,
        body=MappingProxyType(dict(body)),
        source_stage=source_stage,
        trusted=trusted,
        finding_fingerprint=finding_fingerprint,
        provenance=MappingProxyType(dict(provenance)) if provenance is not None else None,
    )


def make_review_memory_ledger(*, scope: ReviewMemoryScope, entries: tuple[ReviewMemoryEntry, ...]) -> ReviewMemoryLedger:
    ledger = ReviewMemoryLedger(scope=scope, entries=entries)
    validate_review_memory_ledger(ledger.to_dict())
    return ledger


def validate_review_memory_entry(payload: Mapping[str, Any]) -> None:
    data = dict(payload)
    validate_required_keys(data, ENTRY_REQUIRED_KEYS, "review memory entry")
    validate_enum(data.get("kind"), REVIEW_MEMORY_KINDS, "review memory entry kind")
    validate_enum(data.get("category"), REVIEW_MEMORY_CATEGORIES, "review memory entry category")
    validate_json_schema(
        {
            "schema_version": SCHEMA_VERSION,
            "scope": {"repository": "validation", "pr_number": 0, "base_ref": "validation"},
            "entries": [data],
        },
        SCHEMA_VERSION,
    )


def is_entry_valid(payload: Mapping[str, Any]) -> bool:
    try:
        validate_review_memory_entry(payload)
    except ValidationError:
        return False
    except Exception as exc:
        if exc.__class__.__module__.startswith("jsonschema"):
            return False
        raise
    return True


def validate_review_memory_ledger(payload: Mapping[str, Any]) -> None:
    data = dict(payload)
    validate_required_keys(data, LEDGER_REQUIRED_KEYS, "review memory ledger")
    require_schema_version(data, SCHEMA_VERSION)
    scope = data.get("scope")
    if not isinstance(scope, dict):
        raise ValidationError("review memory ledger scope must be an object")
    validate_required_keys(scope, SCOPE_REQUIRED_KEYS, "review memory ledger scope")
    entries = data.get("entries")
    if not isinstance(entries, list):
        raise ValidationError("review memory ledger entries must be an array")
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict):
            raise ValidationError(f"review memory ledger entry {index} must be an object")
        validate_enum(entry.get("kind"), REVIEW_MEMORY_KINDS, f"review memory ledger entry {index} kind")
        validate_enum(entry.get("category"), REVIEW_MEMORY_CATEGORIES, f"review memory ledger entry {index} category")
    validate_json_schema(data, SCHEMA_VERSION)
