from __future__ import annotations

from pathlib import Path

import pytest

from codex_review.core.errors import ValidationError
from codex_review.memory.paths import (
    is_memory_path,
    is_safe_memory_path,
    ledger_path,
    memory_root_dir,
    memory_scope_dir,
    notepad_filename,
    notepad_path,
)


def test_scope_and_ledger_paths_are_pr_scoped_and_repo_agnostic() -> None:
    assert memory_root_dir().as_posix() == ".omo/review-memory"
    assert memory_scope_dir("owner/repo", 7).as_posix() == ".omo/review-memory/pr-7"
    assert memory_scope_dir("other/repo", "007").as_posix() == ".omo/review-memory/pr-7"
    assert ledger_path("owner/repo", 7).as_posix() == ".omo/review-memory/pr-7/ledger.json"


@pytest.mark.parametrize(
    ("selector", "filename"),
    [
        ("learning", "learnings.md"),
        ("learnings", "learnings.md"),
        ("decision", "decisions.md"),
        ("open_risk", "issues.md"),
        ("rejected_approach", "problems.md"),
    ],
)
def test_notepad_path_maps_kinds_and_categories_to_four_projection_files(selector: str, filename: str) -> None:
    assert notepad_filename(selector) == filename
    assert notepad_path(selector, "owner/repo", 7).as_posix() == f".omo/review-memory/pr-7/{filename}"


@pytest.mark.parametrize(
    "path",
    [
        ".omo/review-memory/pr-7/ledger.json",
        ".omo/review-memory/pr-7/learnings.md",
        ".omo/review-memory/pr-7/decisions.md",
        ".omo/review-memory/pr-7/issues.md",
        ".omo/review-memory/pr-7/problems.md",
        ".omo/review-memory/pr-7/extra.json",
        ".omo/review-memory/pr-7/nested/ledger.json",
        ".omo/review-memory/pr-7/nested/round-1/scratch.txt",
    ],
)
def test_is_memory_path_accepts_normal_memory_files(path: str) -> None:
    assert is_memory_path(path) is True


@pytest.mark.parametrize(
    "path",
    [
        "src/app.py",
        ".omo/review-memory/../../secrets",
        ".omo/review-memory/pr-7/../ledger.json",
        ".omo/review-memory/pr-7/./ledger.json",
        ".omo/review-memory/pr-7/nested/../ledger.json",
        ".omo/review-memory/pr-7/nested/./ledger.json",
        ".omo/review-memory/pr-7//ledger.json",
        "/repo/.omo/review-memory/pr-7/ledger.json",
        ".omo\\review-memory\\pr-7\\ledger.json",
        ".OMO/review-memory/pr-7/ledger.json",
        ".omo/Review-Memory/pr-7/ledger.json",
        ".omo/review-memory/PR-7/ledger.json",
        ".omo/review-memory/pr-x/ledger.json",
        ".omo/review-memory/pr-007/ledger.json",
        ".omo/review-memory/pr-0/ledger.json",
        ".omo/review-memory/pr-7",
        ".omo/review-memory/pr-7/Ledger.json",
    ],
)
def test_is_memory_path_rejects_source_escape_absolute_case_invalid_scope_and_dot_segments(path: str) -> None:
    assert is_memory_path(path) is False


def test_memory_helpers_reject_invalid_pr_numbers_and_notepad_selectors() -> None:
    with pytest.raises(ValidationError, match="PR number"):
        memory_scope_dir("owner/repo", "../7")
    with pytest.raises(ValidationError, match="positive"):
        ledger_path("owner/repo", 0)
    with pytest.raises(ValidationError, match="unknown memory notepad"):
        notepad_path("routes", "owner/repo", 7)


def test_memory_path_rejects_symlink_scope_without_following_it(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repository_root = tmp_path
    outside_target = repository_root / "outside"
    outside_target.mkdir()
    memory_root = repository_root / ".omo" / "review-memory"
    memory_root.mkdir(parents=True)
    (memory_root / "pr-7").symlink_to(outside_target, target_is_directory=True)
    memory_path = ".omo/review-memory/pr-7/ledger.json"

    monkeypatch.chdir(repository_root)

    assert is_memory_path(memory_path) is False
    assert is_safe_memory_path(memory_path, repository_root) is False


def test_safe_memory_path_rejects_symlink_leaf_under_real_scope(tmp_path: Path) -> None:
    repository_root = tmp_path
    outside_target = repository_root / "outside-ledger.json"
    outside_target.write_text("{}", encoding="utf-8")
    memory_scope = repository_root / ".omo" / "review-memory" / "pr-7"
    memory_scope.mkdir(parents=True)
    (memory_scope / "ledger.json").symlink_to(outside_target)

    assert is_safe_memory_path(".omo/review-memory/pr-7/ledger.json", repository_root) is False
