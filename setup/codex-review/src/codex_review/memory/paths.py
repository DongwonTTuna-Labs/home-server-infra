"""Path helpers for PR-scoped review memory."""
from __future__ import annotations

import os
import re
from pathlib import Path

from codex_review.core.errors import ValidationError
from codex_review.memory.types import CATEGORY_NOTEPAD_FILES

MEMORY_ROOT = Path(".omo") / "review-memory"
MEMORY_ROOT_POSIX = MEMORY_ROOT.as_posix()
LEDGER_FILENAME = "ledger.json"

_KIND_TO_CATEGORY = {
    "fix_applied": "learnings",
    "learning": "learnings",
    "resolved_finding": "learnings",
    "decision": "decisions",
    "open_risk": "issues",
    "rejected_approach": "problems",
}
_GENERATED_MEMORY_FILENAMES = frozenset({LEDGER_FILENAME, *CATEGORY_NOTEPAD_FILES.values()})
_PR_SCOPE_RE = re.compile(r"^pr-([1-9][0-9]*)$")


def memory_root_dir() -> Path:
    return MEMORY_ROOT


def memory_scope_dir(repository: str | None, pr_number: int | str) -> Path:
    _ = repository
    return MEMORY_ROOT / _scope_name(pr_number)


def ledger_path(repository: str | None, pr_number: int | str) -> Path:
    return memory_scope_dir(repository, pr_number) / LEDGER_FILENAME


def notepad_path(kind_or_category: str, repository: str | None, pr_number: int | str) -> Path:
    return memory_scope_dir(repository, pr_number) / notepad_filename(kind_or_category)


def notepad_filename(kind_or_category: str) -> str:
    key = str(kind_or_category or "").strip().lower().replace("-", "_")
    category = _KIND_TO_CATEGORY.get(key, key)
    try:
        return CATEGORY_NOTEPAD_FILES[category]
    except KeyError as exc:
        raise ValidationError(f"unknown memory notepad kind/category: {kind_or_category}") from exc


def is_memory_path(path: str | os.PathLike[str]) -> bool:
    normalized = _normalized_memory_path(path)
    if normalized is None:
        return False
    return not _has_symlink_segment(Path(), normalized.split("/"))


def is_safe_memory_path(path: str | os.PathLike[str], repository_root: str | os.PathLike[str]) -> bool:
    normalized = _normalized_memory_path(path)
    if normalized is None:
        return False
    return not _has_symlink_segment(Path(repository_root), normalized.split("/"))


def is_memory_path_within_repository(path: str | os.PathLike[str], repository_root: str | os.PathLike[str]) -> bool:
    return is_safe_memory_path(path, repository_root)


def _scope_name(pr_number: int | str) -> str:
    if isinstance(pr_number, bool):
        raise ValidationError("memory PR number must be a positive integer")
    text = str(pr_number).strip()
    if not text.isdigit():
        raise ValidationError(f"memory PR number must be numeric: {pr_number}")
    number = int(text)
    if number <= 0:
        raise ValidationError(f"memory PR number must be positive: {pr_number}")
    return f"pr-{number}"


def _normalized_memory_path(path: str | os.PathLike[str]) -> str | None:
    try:
        raw_path = os.fspath(path)
    except TypeError:
        return None
    if not isinstance(raw_path, str):
        return None
    if raw_path != raw_path.strip() or not raw_path:
        return None
    if raw_path.startswith("/") or "\\" in raw_path:
        return None

    parts = raw_path.split("/")
    if any(part in {"", ".", ".."} for part in parts):
        return None
    if len(parts) < 4:
        return None
    if parts[0] != ".omo" or parts[1] != "review-memory":
        return None
    if _PR_SCOPE_RE.fullmatch(parts[2]) is None:
        return None
    if _has_generated_filename_case_variant(parts[3:]):
        return None
    return "/".join(parts)


def _has_generated_filename_case_variant(relative_parts: list[str]) -> bool:
    return any(part.lower() in _GENERATED_MEMORY_FILENAMES and part != part.lower() for part in relative_parts)


def _has_symlink_segment(root: Path, relative_parts: list[str]) -> bool:
    current = root
    for part in relative_parts:
        current = current / part
        try:
            if current.is_symlink():
                return True
        except OSError:
            return True
    return False
