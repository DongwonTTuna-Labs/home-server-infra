"""PR-scoped review-memory merge guard."""
from __future__ import annotations

import subprocess
from pathlib import Path
from typing import Any

from codex_review.core.errors import PolicyViolation, ValidationError
from codex_review.memory.paths import MEMORY_ROOT_POSIX, is_memory_path
from codex_review.security.redaction import safe_log_value
from codex_review.security.subprocess_env import sanitized_env


def assert_review_memory_not_on_base(repo_path: str | Path, base_ref: str = "main") -> dict[str, Any]:
    """Return a clean report or raise when the base tree tracks review memory."""
    normalized_ref = _base_ref(base_ref)
    paths = review_memory_paths_on_ref(repo_path, normalized_ref)
    if paths:
        preview = ", ".join(paths[:10])
        suffix = "" if len(paths) <= 10 else f", ... (+{len(paths) - 10} more)"
        raise PolicyViolation(f"base ref {normalized_ref} contains review-memory paths: {preview}{suffix}")
    return {
        "status": "clean",
        "base_ref": normalized_ref,
        "repo_path": str(Path(repo_path)),
        "memory_path_count": 0,
        "memory_paths": [],
    }


def review_memory_paths_on_ref(repo_path: str | Path, base_ref: str = "main") -> list[str]:
    """List review-memory paths tracked by a git ref without checking it out."""
    repo = Path(repo_path)
    normalized_ref = _base_ref(base_ref)
    proc = subprocess.run(
        [
            "git",
            "-C",
            str(repo),
            "ls-tree",
            "-r",
            "-z",
            "--name-only",
            "--full-tree",
            normalized_ref,
            "--",
            MEMORY_ROOT_POSIX,
        ],
        capture_output=True,
        text=True,
        env=sanitized_env(),
    )
    if proc.returncode != 0:
        detail = safe_log_value(proc.stderr.strip() or f"git ls-tree exited {proc.returncode}")
        raise ValidationError(f"cannot inspect base ref {normalized_ref}: {detail}")

    tracked_paths = [path for path in proc.stdout.split("\0") if path]
    canonical_paths = {path for path in tracked_paths if is_memory_path(path)}
    return sorted(canonical_paths | set(tracked_paths))


def _base_ref(base_ref: str | None) -> str:
    normalized = str(base_ref or "").strip()
    if not normalized:
        raise ValidationError("base ref is required")
    if normalized.startswith("-"):
        raise ValidationError("base ref must be a revision, not an option")
    return normalized
