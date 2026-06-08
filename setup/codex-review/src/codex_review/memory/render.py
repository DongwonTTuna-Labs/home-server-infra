"""Markdown projections and deterministic compaction for review memory."""
from __future__ import annotations

import hashlib
import json
from collections.abc import Iterable, Mapping, Sequence
from copy import deepcopy
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_text
from codex_review.core.config import DEFAULT_CONFIG
from codex_review.core.errors import ValidationError
from codex_review.memory.redaction import redact_memory_text
from codex_review.memory.types import CATEGORY_NOTEPAD_FILES, SCHEMA_VERSION, validate_review_memory_ledger

DEFAULT_BODY_SUMMARY_CHARS = 800
TRUNCATION_MARKER = "...[truncated]"

_CATEGORY_TITLES = {
    "learnings": "Learnings",
    "decisions": "Decisions",
    "issues": "Issues",
    "problems": "Problems",
}
_SUMMARY_KIND_BY_CATEGORY = {
    "learnings": "learning",
    "decisions": "decision",
    "issues": "open_risk",
    "problems": "rejected_approach",
}


def render_projection_files(ledger: Mapping[str, Any], config: Mapping[str, Any] | None = None) -> dict[str, str]:
    """Render the ledger into generated notepad projection markdown by category."""
    payload = _core_ledger(ledger)
    memory = _memory_config(config)
    max_body_chars = _projection_body_chars(memory)
    return _render_projection_files(payload, max_body_chars=max_body_chars)


def render_markdown_files(ledger: Mapping[str, Any], config: Mapping[str, Any] | None = None) -> dict[str, str]:
    """Alias for callers that think in markdown files rather than projections."""
    return render_projection_files(ledger, config)


def compact(ledger: Mapping[str, Any], config: Mapping[str, Any]) -> tuple[dict[str, Any], dict[str, str]]:
    """Compact old review-memory entries and return the compacted ledger plus projections."""
    payload = _core_ledger(ledger)
    memory = _memory_config(config)
    entries = list(payload["entries"])
    max_entries = _positive_int(memory.get("max_entries"), "memory.max_entries")
    keep_recent_rounds = _non_negative_int(memory.get("compaction_keep_recent_rounds"), "memory.compaction_keep_recent_rounds")

    if entries and _needs_compaction(payload, memory, max_entries):
        recent_rounds = _recent_rounds(entries, keep_recent_rounds)
        protected_ids = {
            str(entry["entry_id"])
            for entry in entries
            if entry.get("round") in recent_rounds or _must_preserve_for_suppression(entry)
        }
        summarized_entries = [entry for entry in entries if str(entry["entry_id"]) not in protected_ids]
        kept_entries = [entry for entry in entries if str(entry["entry_id"]) in protected_ids]
        summary_entries = _summary_entries_by_category(summarized_entries)
        next_entries = _oldest_first([*summary_entries, *kept_entries])
        next_entries = _cap_entries(next_entries, max_entries=max_entries, protected_ids=protected_ids)
        payload = {"schema_version": SCHEMA_VERSION, "scope": deepcopy(payload["scope"]), "entries": next_entries}
        _validate_ledger(payload)

    md_files = _render_with_budget(payload, memory)
    return payload, md_files


def write_projection_files(directory: str | Path, md_files: Mapping[str, str]) -> list[Path]:
    """Write generated projection files returned by ``render_projection_files`` or ``compact``."""
    base = Path(directory)
    allowed = set(CATEGORY_NOTEPAD_FILES.values())
    written: list[Path] = []
    for filename in CATEGORY_NOTEPAD_FILES.values():
        if filename not in md_files:
            continue
        if filename not in allowed:
            raise ValidationError(f"unknown memory projection filename: {filename}")
        written.append(write_text(base / filename, md_files[filename]))
    return written


def _core_ledger(ledger: Mapping[str, Any]) -> dict[str, Any]:
    if not isinstance(ledger, Mapping):
        raise ValidationError("review memory ledger must be an object")
    payload = {
        "schema_version": ledger.get("schema_version", SCHEMA_VERSION),
        "scope": deepcopy(ledger.get("scope")),
        "entries": deepcopy(ledger.get("entries", [])),
    }
    _validate_ledger(payload)
    return payload


def _validate_ledger(payload: Mapping[str, Any]) -> None:
    try:
        validate_review_memory_ledger(payload)
    except ValidationError:
        raise
    except Exception as exc:
        if exc.__class__.__module__.startswith("jsonschema"):
            raise ValidationError(f"review memory ledger failed schema validation: {exc}") from exc
        raise


def _memory_config(config: Mapping[str, Any] | None) -> dict[str, Any]:
    default_memory = DEFAULT_CONFIG.get("memory", {})
    memory = dict(default_memory) if isinstance(default_memory, Mapping) else {}
    if config is None:
        return memory
    configured_memory = config.get("memory")
    source = configured_memory if isinstance(configured_memory, Mapping) else config
    memory.update({str(key): value for key, value in source.items()})
    return memory


def _projection_body_chars(memory: Mapping[str, Any]) -> int:
    per_file_budget = int(memory.get("per_file_char_budget", 0) or 0)
    if per_file_budget and per_file_budget < 1200:
        return 240
    return DEFAULT_BODY_SUMMARY_CHARS


def _needs_compaction(ledger: Mapping[str, Any], memory: Mapping[str, Any], max_entries: int) -> bool:
    entries = list(ledger.get("entries", []))
    if len(entries) > max_entries:
        return True
    initial = _render_projection_files(ledger, max_body_chars=DEFAULT_BODY_SUMMARY_CHARS)
    return not _fits_budgets(initial, memory)


def _render_with_budget(ledger: Mapping[str, Any], memory: Mapping[str, Any]) -> dict[str, str]:
    for max_body_chars in (800, 480, 240, 120, 80, 40, 20, 0):
        md_files = _render_projection_files(ledger, max_body_chars=max_body_chars)
        if _fits_budgets(md_files, memory):
            return md_files
    return _render_projection_files(ledger, max_body_chars=0)


def _fits_budgets(md_files: Mapping[str, str], memory: Mapping[str, Any]) -> bool:
    per_file_budget = _positive_int(memory.get("per_file_char_budget"), "memory.per_file_char_budget")
    total_budget = _positive_int(memory.get("total_char_budget"), "memory.total_char_budget")
    return all(len(text) <= per_file_budget for text in md_files.values()) and sum(len(text) for text in md_files.values()) <= total_budget


def _render_projection_files(ledger: Mapping[str, Any], *, max_body_chars: int) -> dict[str, str]:
    entries = list(ledger.get("entries", []))
    return {
        filename: _render_category_file(category, entries, max_body_chars=max_body_chars)
        for category, filename in CATEGORY_NOTEPAD_FILES.items()
    }


def _render_category_file(category: str, entries: Sequence[Mapping[str, Any]], *, max_body_chars: int) -> str:
    category_entries = _newest_first([entry for entry in entries if entry.get("category") == category])
    title = _CATEGORY_TITLES[category]
    lines = [
        f"# {title}",
        "",
        "Generated from `review-memory.v1` ledger. Do not edit this projection manually.",
        f"Category: `{category}`",
        f"Entries: {len(category_entries)}",
        "",
    ]
    if not category_entries:
        lines.append("No entries.")
        return "\n".join(lines).rstrip() + "\n"

    for entry in category_entries:
        lines.extend(_render_entry(entry, max_body_chars=max_body_chars))
    return "\n".join(lines).rstrip() + "\n"


def _render_entry(entry: Mapping[str, Any], *, max_body_chars: int) -> list[str]:
    trust = "trusted" if entry.get("trusted") is True else "advisory"
    entry_id = _safe_inline_metadata(entry.get("entry_id"))
    kind = _safe_inline_metadata(entry.get("kind"))
    round_value = _safe_inline_metadata(entry.get("round"))
    head_sha = _safe_inline_metadata(entry.get("head_sha"))
    source_stage = _safe_inline_metadata(entry.get("source_stage"))
    created_at = _safe_inline_metadata(entry.get("created_at"))
    lines = [
        f"## {entry_id}",
        f"- kind: `{kind}`",
        f"- round: {round_value}",
        f"- head_sha: `{head_sha}`",
        f"- source_stage: `{source_stage}`",
        f"- status: {trust}",
        f"- created_at: `{created_at}`",
    ]
    fingerprint = _safe_inline_metadata(entry.get("finding_fingerprint") or "")
    if fingerprint:
        lines.append(f"- fingerprint: `{fingerprint}`")
    lines.extend(["", f"Body: {_body_summary(entry.get('body', {}), max_body_chars)}", ""])
    return lines


def _safe_inline_metadata(value: Any) -> str:
    text = redact_memory_text("" if value is None else str(value))
    return " ".join(text.replace("`", "'").split())


def _body_summary(body: Any, max_chars: int) -> str:
    try:
        json_text = json.dumps(body, ensure_ascii=False, sort_keys=True, separators=(",", ":"), default=str)
    except TypeError:
        json_text = str(body)
    if isinstance(body, Mapping) and "summary" in body:
        text = f"{body.get('summary')} | {json_text}"
    else:
        text = json_text
    sanitized = redact_memory_text(" ".join(text.split()))
    return _truncate(sanitized, max_chars)


def _truncate(text: str, max_chars: int) -> str:
    if max_chars <= 0:
        return TRUNCATION_MARKER
    if len(text) <= max_chars:
        return text
    if max_chars <= len(TRUNCATION_MARKER):
        return TRUNCATION_MARKER[:max_chars]
    return text[: max_chars - len(TRUNCATION_MARKER)].rstrip() + TRUNCATION_MARKER


def _summary_entries_by_category(entries: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    summaries: list[dict[str, Any]] = []
    for category in CATEGORY_NOTEPAD_FILES:
        category_entries = [entry for entry in entries if entry.get("category") == category]
        if category_entries:
            summaries.append(_summary_entry(category, category_entries))
    return summaries


def _summary_entry(category: str, entries: Sequence[Mapping[str, Any]]) -> dict[str, Any]:
    ordered = _oldest_first(entries)
    rounds = [int(entry["round"]) for entry in ordered]
    created_values = [str(entry.get("created_at") or "") for entry in ordered]
    body = {
        "compacted": True,
        "summary": _summary_sentence(category, len(ordered), min(rounds), max(rounds)),
        "entry_count": len(ordered),
        "rounds": {"min": min(rounds), "max": max(rounds)},
        "kinds": _counts(entry.get("kind") for entry in ordered),
        "categories": {category: len(ordered)},
        "source_stages": _counts(entry.get("source_stage") for entry in ordered),
    }
    digest = hashlib.sha256(json.dumps(body, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()[:12]
    return {
        "entry_id": f"compacted-{category}-r{min(rounds)}-r{max(rounds)}-{digest}",
        "created_at": max(created_values) if created_values else "1970-01-01T00:00:00Z",
        "round": max(rounds),
        "head_sha": f"compacted:{digest}",
        "kind": _SUMMARY_KIND_BY_CATEGORY[category],
        "category": category,
        "body": body,
        "source_stage": "memory_compaction",
        "trusted": False,
    }


def _summary_sentence(category: str, count: int, min_round: int, max_round: int) -> str:
    round_range = str(min_round) if min_round == max_round else f"{min_round}-{max_round}"
    return f"Compacted {count} older {category} entries from round(s) {round_range}."


def _counts(values: Iterable[Any]) -> dict[str, int]:
    counts: dict[str, int] = {}
    for value in values:
        key = str(value or "")
        counts[key] = counts.get(key, 0) + 1
    return {key: counts[key] for key in sorted(counts)}


def _recent_rounds(entries: Sequence[Mapping[str, Any]], keep_recent_rounds: int) -> set[int]:
    if keep_recent_rounds <= 0:
        return set()
    rounds = sorted({int(entry["round"]) for entry in entries}, reverse=True)
    return set(rounds[:keep_recent_rounds])


def _must_preserve_for_suppression(entry: Mapping[str, Any]) -> bool:
    return (
        entry.get("kind") == "resolved_finding"
        and entry.get("trusted") is True
        and bool(str(entry.get("finding_fingerprint") or "").strip())
    )


def _cap_entries(entries: Sequence[Mapping[str, Any]], *, max_entries: int, protected_ids: set[str]) -> list[dict[str, Any]]:
    copied = [deepcopy(dict(entry)) for entry in entries]
    if len(copied) <= max_entries:
        return copied
    drop_count = len(copied) - max_entries
    droppable = [entry for entry in copied if str(entry.get("entry_id")) not in protected_ids]
    drop_ids = {str(entry.get("entry_id")) for entry in droppable[:drop_count]}
    return [entry for entry in copied if str(entry.get("entry_id")) not in drop_ids]


def _newest_first(entries: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    return [deepcopy(dict(entry)) for entry in sorted(entries, key=_entry_sort_key, reverse=True)]


def _oldest_first(entries: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    return [deepcopy(dict(entry)) for entry in sorted(entries, key=_entry_sort_key)]


def _entry_sort_key(entry: Mapping[str, Any]) -> tuple[int, str, str]:
    return (int(entry.get("round", 0)), str(entry.get("created_at") or ""), str(entry.get("entry_id") or ""))


def _positive_int(value: Any, name: str) -> int:
    try:
        normalized = int(value)
    except (TypeError, ValueError):
        raise ValidationError(f"{name} must be positive") from None
    if normalized <= 0:
        raise ValidationError(f"{name} must be positive")
    return normalized


def _non_negative_int(value: Any, name: str) -> int:
    try:
        normalized = int(value)
    except (TypeError, ValueError):
        raise ValidationError(f"{name} must be non-negative") from None
    if normalized < 0:
        raise ValidationError(f"{name} must be non-negative")
    return normalized
