"""Convert structured search/replace edits into a git-generated unified diff.

The fix model emits ``edits: [{path, old_str, new_str}]`` instead of a raw
unified-diff string. LLMs reliably produce correct search/replace blocks but
routinely miscount unified-diff hunk headers ("corrupt patch at line N"). We
apply the edits to a throwaway clone of the repo and let ``git diff`` produce
the patch, so the resulting patch is always well-formed and applicable.
"""
from __future__ import annotations

import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from codex_review.core.errors import ValidationError
from codex_review.security.subprocess_env import sanitized_env


def _apply_one_edit(root: Path, edit: dict[str, Any]) -> None:
    path = edit.get("path")
    if not path or not isinstance(path, str):
        raise ValidationError("edit missing path")
    old = edit.get("old_str") or ""
    new = edit.get("new_str") or ""
    target = (root / path)
    # Guard against path escapes from model output.
    if not str(target.resolve()).startswith(str(root.resolve())):
        raise ValidationError(f"edit path escapes repository: {path}")
    if old == "":
        if target.exists() and target.read_text(encoding="utf-8", errors="replace") != "":
            raise ValidationError(f"edit with empty old_str targets existing non-empty file: {path}")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(new, encoding="utf-8")
        return
    if not target.exists():
        raise ValidationError(f"edit target file not found: {path}")
    content = target.read_text(encoding="utf-8")
    occurrences = content.count(old)
    if occurrences == 0:
        raise ValidationError(f"edit old_str not found in {path}")
    if occurrences > 1:
        raise ValidationError(f"edit old_str not unique in {path} ({occurrences} matches); add surrounding context")
    target.write_text(content.replace(old, new, 1), encoding="utf-8")


def _delete_one(root: Path, path: str) -> None:
    if not path or not isinstance(path, str):
        raise ValidationError("deletion path must be a string")
    target = root / path
    if not str(target.resolve()).startswith(str(root.resolve())):
        raise ValidationError(f"deletion path escapes repository: {path}")
    if target.exists():
        target.unlink()  # absent target is a no-op (already in desired state)


def apply_edits_and_generate_patch(edits: list[dict[str, Any]], repo_path: str | Path, deletions: list[str] | None = None) -> str:
    """Apply search/replace edits and file deletions in a temp clone; return a `git diff` patch."""
    edits = edits or []
    deletions = deletions or []
    if not edits and not deletions:
        return ""
    repo = Path(repo_path)
    tmp = Path(tempfile.mkdtemp(prefix="codex-edits-"))
    try:
        if (repo / ".git").exists():
            subprocess.run(["git", "clone", "--quiet", repo.as_posix(), tmp.as_posix()], check=True, env=sanitized_env())
        else:
            shutil.copytree(repo, tmp, dirs_exist_ok=True)
        for edit in edits:
            if not isinstance(edit, dict):
                raise ValidationError("edit must be an object with path/old_str/new_str")
            _apply_one_edit(tmp, edit)
        for path in deletions:
            _delete_one(tmp, path)
        # Stage everything so new + deleted files appear in the diff, then diff HEAD..index.
        subprocess.run(["git", "add", "-A"], cwd=tmp, check=True, env=sanitized_env())
        proc = subprocess.run(["git", "diff", "--binary", "--staged"], cwd=tmp, capture_output=True, text=True, env=sanitized_env())
        if proc.returncode != 0:
            raise ValidationError(f"failed to generate patch from edits: {proc.stderr.strip()}")
        return proc.stdout
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def ensure_patch_from_edits(obj: dict[str, Any], repo_path: str | Path | None) -> dict[str, Any]:
    """If a fix artifact carries structured `edits`, materialize `patch` from them.

    Edits are authoritative when present. No-op when there are no edits (the
    deterministic clean-merge path already produces a valid `patch`).
    """
    if not isinstance(obj, dict):
        return obj
    edits = obj.get("edits")
    deletions = obj.get("deletions")
    if (edits or deletions) and repo_path is not None:
        patch = apply_edits_and_generate_patch(edits, repo_path, deletions)
        obj["patch"] = patch
        obj["patch_text"] = patch
    return obj
