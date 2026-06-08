from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

from codex_review.cli import main
from codex_review.memory.paths import ledger_path
from codex_review.memory.provenance import HMAC_KEY_ENV
from codex_review.memory.writer import (
    MEMORY_COMMIT_TRAILER,
    build_memory_commit_message,
    build_terminal_memory_entry,
    terminal_memory_status,
    write_terminal_memory_commit,
)
from codex_review.memory import writer
from codex_review.stages.reentry import record as reentry_record


def _fake_secret(prefix: str = "ghp_") -> str:
    return prefix + "Aa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8Ii9Jj0KkLlMmNnPpQqRrSsTt9"


def _git(repo: Path, *args: str, input_text: str | None = None) -> str:
    return subprocess.check_output(["git", *args], cwd=repo, input=input_text, text=True).strip()


def _init_repo(tmp_path: Path, name: str = "repo") -> tuple[Path, str]:
    repo = tmp_path / name
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    subprocess.run(["git", "add", "README.md"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    head = _git(repo, "rev-parse", "HEAD")
    subprocess.run(["git", "remote", "add", "origin", "https://example.invalid/owner/repo.git"], cwd=repo, check=True)
    return repo, head


def _pr_context(head: str) -> dict:
    return {
        "owner": "owner",
        "repo": "repo",
        "repository": "owner/repo",
        "base_repo_full_name": "owner/repo",
        "head_repo_full_name": "owner/repo",
        "same_repo": True,
        "pr_number": 7,
        "base_ref": "main",
        "head_ref": "feature/memory",
        "head_sha": head,
    }


def _terminal_record(status: str, head: str, *, reason: str = "terminal outcome") -> dict:
    return {
        "schema_version": "reentry-loop-state.v1",
        "pushed": False,
        "next_entry": "none",
        "loop_state": {"round_count": 2, "head_sha": head},
        "artifacts": {"push_result": {"status": status, "head_sha": head, "reason": reason}},
    }


def test_lgtm_and_no_fix_terminal_entries_are_signed_and_redacted(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "testkey")
    secret = _fake_secret()
    ctx = _pr_context("abc123")

    lgtm_entry = build_terminal_memory_entry({"lgtm": True, "pushed": False, "head_sha": "abc123"}, ctx, created_at="2026-06-08T00:00:00Z")
    no_fix_entry = build_terminal_memory_entry(_terminal_record("no_fix", "abc123", reason=f"no patch with {secret}"), ctx, created_at="2026-06-08T00:00:00Z")

    assert terminal_memory_status({"lgtm": True, "pushed": False}) == "lgtm"
    assert terminal_memory_status(_terminal_record("empty_patch", "abc123")) == "no_fix_needed"
    assert lgtm_entry["trusted"] is True
    assert no_fix_entry["trusted"] is True
    assert lgtm_entry["body"]["terminal_status"] == "lgtm"
    assert no_fix_entry["body"]["terminal_status"] == "no_fix_needed"
    encoded = json.dumps(no_fix_entry, sort_keys=True)
    assert secret not in encoded
    assert "[REDACTED_SECRET]" in encoded


def test_terminal_no_fix_memory_commit_writes_only_memory_paths_and_pushes(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "testkey")
    repo, head = _init_repo(tmp_path)
    captured: dict[str, str] = {}

    def fake_push(repo_path: Path, head_ref: str, owner: str, repo_name: str, token: str | None) -> dict:
        sha = _git(Path(repo_path), "rev-parse", "HEAD")
        captured.update({"head_ref": head_ref, "owner": owner, "repo": repo_name, "token": token or "", "sha": sha})
        return {"pushed": True, "verified": True, "remote_head_sha": sha, "expected_head_sha": sha}

    monkeypatch.setattr(writer, "push_commit", fake_push)

    result = write_terminal_memory_commit(_terminal_record("no_fix_changes", head, reason="patch already present"), _pr_context(head), "token", repo)

    assert result["status"] == "pushed"
    assert result["memory_only"] is True
    assert result["verified"] is True
    assert captured == {"head_ref": "feature/memory", "owner": "owner", "repo": "repo", "token": "token", "sha": result["commit_sha"]}
    assert result["commit_message_trailer"] == MEMORY_COMMIT_TRAILER

    ledger = json.loads((repo / ledger_path("owner/repo", 7)).read_text(encoding="utf-8"))
    assert ledger["entries"][0]["source_stage"] == "reentry_terminal_memory"
    assert ledger["entries"][0]["body"]["terminal_status"] == "no_fix_changes"
    assert ledger["entries"][0]["trusted"] is True
    assert (repo / ".omo/review-memory/pr-7/learnings.md").exists()

    message = _git(repo, "log", "-1", "--format=%B")
    assert MEMORY_COMMIT_TRAILER in message
    assert "[skip ci]" not in message.lower()
    assert "skip-checks" not in message.lower()
    changed_paths = [line for line in _git(repo, "show", "--name-only", "--format=", "HEAD").splitlines() if line]
    assert changed_paths
    assert all(path.startswith(".omo/review-memory/pr-7/") for path in changed_paths)


def test_reentry_record_cli_uses_repo_path_not_current_working_directory(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "testkey")
    repo, head = _init_repo(tmp_path, "pr-head")
    trusted_cwd, _trusted_head = _init_repo(tmp_path, "trusted-core")
    (trusted_cwd / "dirty.txt").write_text("dirty trusted checkout\n", encoding="utf-8")
    captured: dict[str, str] = {}

    def fake_push(repo_path: Path, head_ref: str, owner: str, repo_name: str, token: str | None) -> dict:
        sha = _git(Path(repo_path), "rev-parse", "HEAD")
        captured.update({"repo_path": str(repo_path), "head_ref": head_ref, "owner": owner, "repo": repo_name, "token": token or "", "sha": sha})
        return {"pushed": True, "verified": True, "remote_head_sha": sha, "expected_head_sha": sha}

    push_result_path = tmp_path / "push-result.json"
    pr_context_path = tmp_path / "pr-context.json"
    out_path = tmp_path / "reentry.json"
    push_result_path.write_text(
        json.dumps({"schema_version": "push-result.v1", "status": "no_fix", "pushed": False, "head_sha": head, "reason": "CLI no-fix smoke"}),
        encoding="utf-8",
    )
    pr_context_path.write_text(json.dumps(_pr_context(head)), encoding="utf-8")

    monkeypatch.chdir(trusted_cwd)
    monkeypatch.setattr(writer, "push_commit", fake_push)

    rc = main(
        [
            "reentry",
            "record-reentry",
            "--in",
            str(push_result_path),
            "--pr-context",
            str(pr_context_path),
            "--repo-path",
            str(repo),
            "--token",
            "token",
            "--out",
            str(out_path),
        ]
    )

    payload = json.loads(out_path.read_text(encoding="utf-8"))
    assert rc == 0
    assert payload["pushed"] is False
    assert payload["next_entry"] == "none"
    assert payload["memory_write"]["status"] == "pushed"
    assert captured["repo_path"] == str(repo)
    assert (repo / ledger_path("owner/repo", 7)).exists()
    assert (repo / ".omo/review-memory/pr-7/learnings.md").exists()
    assert not (trusted_cwd / ".omo").exists()


def test_reentry_memory_write_failure_is_nonfatal_and_preserves_terminal_outcome(monkeypatch: pytest.MonkeyPatch) -> None:
    secret = _fake_secret()

    def boom(*args, **kwargs):
        raise RuntimeError(f"writer failed with {secret}")

    monkeypatch.setattr(reentry_record, "write_terminal_memory_commit", boom)
    record = _terminal_record("no_fix", "abc123")

    out = reentry_record.persist_reentry_loop_state(record, _pr_context("abc123"), "token")

    assert out["pushed"] is False
    assert out["next_entry"] == "none"
    assert out["persisted"] is False
    assert out["persist_reason"] == "no push occurred"
    assert out["memory_write"]["status"] == "failed"
    assert out["memory_write"]["non_fatal"] is True
    assert out["memory_write"]["error_type"] == "RuntimeError"
    assert secret not in out["memory_write"]["error"]
    assert "[REDACTED_SECRET]" in out["memory_write"]["error"]


def test_fork_pr_skips_terminal_memory_commit_without_writing(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    repo, head = _init_repo(tmp_path)

    def fail_push(*args, **kwargs):
        raise AssertionError("fork PR must not push")

    monkeypatch.setattr(writer, "push_commit", fail_push)
    ctx = {**_pr_context(head), "same_repo": False, "head_repo_full_name": "fork/repo"}

    result = write_terminal_memory_commit(_terminal_record("no_fix", head), ctx, "token", repo)

    assert result["status"] == "skipped"
    assert "unsafe_write_context" in result["reason"]
    assert not (repo / ".omo").exists()


def test_memory_commit_message_has_audit_trailer_without_ci_skip_marker() -> None:
    message = build_memory_commit_message({"entry_id": "terminal-no-fix-needed-r2-abc123"})

    assert message.endswith(f"{MEMORY_COMMIT_TRAILER}\n")
    assert "[skip ci]" not in message.lower()
    assert "skip-checks" not in message.lower()
