from __future__ import annotations

import json
import subprocess
from pathlib import Path

from codex_review.cli import main
from codex_review.memory.merge_guard import assert_review_memory_not_on_base, review_memory_paths_on_ref
from codex_review.stages.push.commit import create_commits_from_plan

MEMORY_PATH = ".omo/review-memory/pr-7/ledger.json"


def _run_git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()


def _ls_tree(repo: Path, ref: str, pathspec: str) -> list[str]:
    out = subprocess.check_output(["git", "ls-tree", "-r", "--name-only", ref, "--", pathspec], cwd=repo, text=True)
    return [line for line in out.splitlines() if line]


def _init_repo(tmp_path: Path) -> tuple[Path, str]:
    repo = tmp_path / "repo"
    repo.mkdir()
    _run_git(repo, "init")
    _run_git(repo, "config", "user.name", "Test")
    _run_git(repo, "config", "user.email", "test@example.invalid")
    (repo / ".gitignore").write_text(".omo/\n", encoding="utf-8")
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _run_git(repo, "add", ".gitignore", "README.md")
    _run_git(repo, "commit", "-m", "base")
    _run_git(repo, "branch", "-M", "main")
    return repo, _git(repo, "rev-parse", "HEAD")


def _write_memory_file(repo: Path, content: str = "{}\n") -> None:
    path = repo / MEMORY_PATH
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def test_assert_not_on_base_passes_when_main_has_no_review_memory(tmp_path: Path) -> None:
    repo, _base = _init_repo(tmp_path)
    out_path = tmp_path / "guard.json"

    rc = main(["memory", "assert-not-on-base", "--base-ref", "main", "--repo-path", str(repo), "--out", str(out_path)])

    payload = json.loads(out_path.read_text(encoding="utf-8"))
    assert rc == 0
    assert payload == {"base_ref": "main", "memory_path_count": 0, "memory_paths": [], "repo_path": str(repo), "status": "clean"}
    assert review_memory_paths_on_ref(repo, "main") == []
    ignored = subprocess.run(["git", "check-ignore", "--no-index", MEMORY_PATH], cwd=repo, capture_output=True, text=True)
    assert ignored.returncode == 0


def test_assert_not_on_base_flags_review_memory_tracked_on_main(tmp_path: Path, capsys) -> None:
    repo, _base = _init_repo(tmp_path)
    _write_memory_file(repo)
    _run_git(repo, "add", "-f", MEMORY_PATH)
    _run_git(repo, "commit", "-m", "bad base memory")
    out_path = tmp_path / "blocked.json"

    rc = main(["memory", "assert-not-on-base", "--base-ref", "main", "--repo-path", str(repo), "--out", str(out_path)])
    captured = capsys.readouterr()

    assert rc == 2
    assert not out_path.exists()
    assert review_memory_paths_on_ref(repo, "main") == [MEMORY_PATH]
    assert "PolicyViolation" in captured.err
    assert MEMORY_PATH in captured.err


def test_trusted_force_add_can_commit_pr_memory_while_main_stays_clean(tmp_path: Path) -> None:
    repo, base = _init_repo(tmp_path)
    _run_git(repo, "checkout", "-b", "feature/memory")
    (repo / "README.md").write_text("base\nfeature\n", encoding="utf-8")
    _write_memory_file(repo, '{"schema_version":"review-memory.v1","entries":[]}\n')
    plan = [{"subject": "test: update with memory sidecar", "body": "Update code and sidecar.", "paths": ["README.md", MEMORY_PATH]}]

    shas = create_commits_from_plan(
        repo,
        plan,
        "plan-hash",
        base,
        {"commit_plan": plan},
        trusted_force_add_paths=[MEMORY_PATH],
        trusted_force_add_prefix=".omo/review-memory/",
    )

    assert shas
    assert assert_review_memory_not_on_base(repo, "main")["status"] == "clean"
    assert _ls_tree(repo, "main", ".omo/review-memory") == []
    assert MEMORY_PATH in _ls_tree(repo, "HEAD", ".omo/review-memory")
    committed_paths = _ls_tree(repo, "HEAD", ".")
    assert "README.md" in committed_paths
    assert MEMORY_PATH in committed_paths
