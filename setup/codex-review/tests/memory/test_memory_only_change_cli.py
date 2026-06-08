from __future__ import annotations

import json
import subprocess
from pathlib import Path

from codex_review.cli import main


MEMORY_COMMIT_TRAILER = "codex-memory: true"
OWN_ACTOR = "github-actions[bot]"


def _git(repo: Path, *args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=repo, text=True).strip()


def _run_git(repo: Path, *args: str) -> None:
    subprocess.run(["git", *args], cwd=repo, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def _init_repo(tmp_path: Path) -> tuple[Path, str]:
    repo = tmp_path / "repo"
    repo.mkdir()
    _run_git(repo, "init")
    _run_git(repo, "config", "user.name", "Test")
    _run_git(repo, "config", "user.email", "test@example.invalid")
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    _run_git(repo, "add", "README.md")
    _run_git(repo, "commit", "-m", "base")
    return repo, _git(repo, "rev-parse", "HEAD")


def _commit(repo: Path, files: dict[str, str], message: str) -> str:
    for relative, content in files.items():
        path = repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    _run_git(repo, "add", "-A")
    _run_git(repo, "add", "-f", *files)
    _run_git(repo, "commit", "-m", message)
    return _git(repo, "rev-parse", "HEAD")


def _classify(
    repo: Path,
    base: str | None,
    head: str | None,
    tmp_path: Path,
    monkeypatch,
    *,
    actor: str | None = None,
    requested_by: str | None = None,
    own_actors: str | None = None,
) -> tuple[dict, str]:
    out_path = tmp_path / f"classification-{len(list(tmp_path.glob('classification-*.json')))}.json"
    github_output = tmp_path / f"github-output-{out_path.stem}.txt"
    monkeypatch.setenv("GITHUB_OUTPUT", str(github_output))
    for name in ("CODEX_LOOP_GITHUB_ACTOR", "GITHUB_ACTOR", "CODEX_LOOP_REQUESTED_BY", "CODEX_LOOP_OWN_ACTORS"):
        monkeypatch.delenv(name, raising=False)
    if actor is not None:
        monkeypatch.setenv("CODEX_LOOP_GITHUB_ACTOR", actor)
    if requested_by is not None:
        monkeypatch.setenv("CODEX_LOOP_REQUESTED_BY", requested_by)
    if own_actors is not None:
        monkeypatch.setenv("CODEX_LOOP_OWN_ACTORS", own_actors)

    argv = ["loop", "memory-only-change", "--repo-path", str(repo), "--out", str(out_path)]
    if base is not None:
        argv.extend(["--base", base])
    if head is not None:
        argv.extend(["--head", head])

    rc = main(argv)

    assert rc == 0
    return json.loads(out_path.read_text(encoding="utf-8")), github_output.read_text(encoding="utf-8")


def test_memory_only_diff_sets_no_model_for_tagged_review_memory_from_own_actor(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(
        repo,
        {
            ".omo/review-memory/pr-7/ledger.json": "{}\n",
            ".omo/review-memory/pr-7/generated/round-1/context.json": "{}\n",
        },
        f"memory\n\n{MEMORY_COMMIT_TRAILER}",
    )

    payload, outputs = _classify(repo, base, head, tmp_path, monkeypatch, actor=OWN_ACTOR, requested_by=OWN_ACTOR, own_actors=OWN_ACTOR)

    assert payload["memory_only"] is True
    assert payload["should_run_model"] is False
    assert payload["reason"] == "memory_only"
    assert payload["codex_memory_marker"] is True
    assert payload["actor_guard"] is True
    assert payload["non_memory_paths"] == []
    assert ".omo/review-memory/pr-7/generated/round-1/context.json" in payload["changed_paths"]
    assert "memory_only=true" in outputs
    assert "should_run_model=false" in outputs
    assert "codex_memory_marker=true" in outputs
    assert "actor_guard=true" in outputs


def test_memory_only_diff_without_codex_memory_marker_fails_open(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(repo, {".omo/review-memory/pr-7/ledger.json": "{}\n"}, "memory")

    payload, outputs = _classify(repo, base, head, tmp_path, monkeypatch, actor=OWN_ACTOR, requested_by=OWN_ACTOR, own_actors=OWN_ACTOR)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["reason"] == "missing_codex_memory_marker"
    assert payload["codex_memory_marker"] is False
    assert payload["actor_guard"] is False
    assert "memory_only=false" in outputs
    assert "should_run_model=true" in outputs
    assert "codex_memory_marker=false" in outputs


def test_memory_only_marker_without_own_actor_guard_fails_open(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(repo, {".omo/review-memory/pr-7/ledger.json": "{}\n"}, f"memory\n\n{MEMORY_COMMIT_TRAILER}")

    payload, outputs = _classify(repo, base, head, tmp_path, monkeypatch, actor="octocat", requested_by="octocat", own_actors=OWN_ACTOR)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["reason"] == "actor_guard_mismatch"
    assert payload["codex_memory_marker"] is True
    assert payload["actor_guard"] is False
    assert "actor_guard=false" in outputs


def test_missing_actor_guard_configuration_fails_open(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(repo, {".omo/review-memory/pr-7/ledger.json": "{}\n"}, f"memory\n\n{MEMORY_COMMIT_TRAILER}")

    payload, _outputs = _classify(repo, base, head, tmp_path, monkeypatch, actor=OWN_ACTOR, requested_by=OWN_ACTOR)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["reason"] == "missing_actor_guard"
    assert payload["codex_memory_marker"] is True
    assert payload["actor_guard"] is False


def test_code_only_diff_runs_model(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(repo, {"src/app.py": "print('changed')\n"}, "code")

    payload, outputs = _classify(repo, base, head, tmp_path, monkeypatch)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["non_memory_paths"] == ["src/app.py"]
    assert "memory_only=false" in outputs
    assert "should_run_model=true" in outputs


def test_code_and_memory_diff_runs_model(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(
        repo,
        {
            ".omo/review-memory/pr-7/learnings.md": "# Learnings\n",
            "src/app.py": "print('changed')\n",
        },
        "mixed",
    )

    payload, _outputs = _classify(repo, base, head, tmp_path, monkeypatch)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["reason"] == "contains_non_memory_changes"
    assert payload["non_memory_paths"] == ["src/app.py"]


def test_other_omo_paths_are_not_memory_only(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(repo, {".omo/notepads/codex-loop-pr-memory/learnings.md": "note\n"}, "notepad")

    payload, _outputs = _classify(repo, base, head, tmp_path, monkeypatch)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["non_memory_paths"] == [".omo/notepads/codex-loop-pr-memory/learnings.md"]


def test_empty_diff_does_not_suppress_model(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)

    payload, _outputs = _classify(repo, base, base, tmp_path, monkeypatch)

    assert payload["memory_only"] is False
    assert payload["should_run_model"] is True
    assert payload["reason"] == "empty_diff"
    assert payload["changed_paths"] == []


def test_invalid_or_missing_base_head_fails_open_to_model(tmp_path: Path, monkeypatch) -> None:
    repo, base = _init_repo(tmp_path)
    head = _commit(repo, {".omo/review-memory/pr-7/ledger.json": "{}\n"}, "memory")

    invalid_payload, _invalid_outputs = _classify(repo, "missing-base", head, tmp_path, monkeypatch)
    missing_payload, _missing_outputs = _classify(repo, base, None, tmp_path, monkeypatch)

    assert invalid_payload["memory_only"] is False
    assert invalid_payload["should_run_model"] is True
    assert invalid_payload["reason"] == "diff_unavailable"
    assert missing_payload["memory_only"] is False
    assert missing_payload["should_run_model"] is True
    assert missing_payload["reason"] == "missing_base_or_head"
