from __future__ import annotations

import hashlib
import json
import subprocess
from pathlib import Path

import pytest

from codex_review.core.errors import ValidationError
from codex_review.memory.provenance import HMAC_KEY_ENV
from codex_review.memory.writer import write_trusted_push_memory_sidecar
from codex_review.stages.push import orchestrate
from codex_review.stages.push.apply_patch import collect_applied_diff
from codex_review.stages.push.commit import create_commits_from_plan
from codex_review.stages.push.orchestrate import commit_and_push_validated_fix


PATCH = """diff --git a/docs/a.md b/docs/a.md
--- a/docs/a.md
+++ b/docs/a.md
@@ -1 +1 @@
-old
+new
"""

MEMORY_PATHS = [
    ".omo/review-memory/pr-7/ledger.json",
    ".omo/review-memory/pr-7/learnings.md",
    ".omo/review-memory/pr-7/decisions.md",
    ".omo/review-memory/pr-7/issues.md",
    ".omo/review-memory/pr-7/problems.md",
]


def _init_repo(tmp_path: Path) -> tuple[Path, str]:
    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    subprocess.run(["git", "config", "user.name", "Test"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.email", "test@example.invalid"], cwd=repo, check=True)
    (repo / ".gitignore").write_text(".omo/\n", encoding="utf-8")
    (repo / "docs").mkdir()
    (repo / "docs/a.md").write_text("old\n", encoding="utf-8")
    subprocess.run(["git", "add", ".gitignore", "docs/a.md"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, stdout=subprocess.DEVNULL)
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    return repo, head


def _config() -> dict:
    return {
        "base_branch": "main",
        "memory": {
            "enabled": True,
            "root": ".omo/review-memory",
            "ledger_file": "ledger.json",
            "notepad_files": ["learnings.md", "decisions.md", "issues.md", "problems.md"],
            "max_entries": 200,
            "per_file_char_budget": 6000,
            "total_char_budget": 24000,
            "compaction_keep_recent_rounds": 3,
            "provenance_required_for_suppression": True,
            "hmac_env": HMAC_KEY_ENV,
        },
        "autofix": {"allowed_prefixes": ["docs/"], "memory_write_prefix": ".omo/review-memory/", "max_patch_bytes": 20000},
    }


def _commit_plan() -> list[dict]:
    return [{"subject": "docs(a): update trusted sidecar fixture", "body": "Update the docs fixture.", "paths": ["docs/a.md"]}]


def _validation(patch: str, applied_diff_hash: str, *, secret: str = "") -> dict:
    test_status = f"passed {secret}" if secret else "passed"
    return {
        "schema_version": "push-validated-fix.v1",
        "status": "validated",
        "validated": True,
        "semantic_safety_approved": True,
        "patch_hash": hashlib.sha256(patch.encode("utf-8")).hexdigest(),
        "applied_diff_hash": applied_diff_hash,
        "head_sha": "validated-head",
        "test_report": {"passed": True, "status": test_status},
        "semantic_safety": {"status": "approved", "approved": True, "commit_plan": _commit_plan()},
    }


def _pr(head: str) -> dict:
    return {
        "owner": "owner",
        "repo": "repo",
        "repository": "owner/repo",
        "base_ref": "main",
        "pr_number": 7,
        "head_sha": head,
        "head_ref": "feature/sidecar",
        "same_repo": True,
    }


def _preapply_patch(repo: Path) -> str:
    subprocess.run(["git", "apply", "--index", "-"], input=PATCH, text=True, cwd=repo, check=True)
    return hashlib.sha256(collect_applied_diff(repo).encode("utf-8")).hexdigest()


def _stub_push(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(orchestrate, "validate_current_head", lambda *args, **kwargs: None)
    monkeypatch.setattr(orchestrate, "push_commit", lambda *args, **kwargs: {"pushed": True, "returncode": 0, "verified": True})
    monkeypatch.setattr(orchestrate, "verify_pushed_head", lambda *args, **kwargs: True)


def test_commit_push_writes_trusted_memory_sidecar_into_same_fix_commit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "task-16-test-key")
    repo, head = _init_repo(tmp_path)
    applied_hash = _preapply_patch(repo)
    _stub_push(monkeypatch)
    fake_secret = "github_pat_" + "A" * 82

    result = commit_and_push_validated_fix(
        {"schema_version": "fix-merge-merged-fix.v1", "status": "ready_to_push", "patch": PATCH, "expected_head_sha": head, "round": 3},
        _validation(PATCH, applied_hash, secret=fake_secret),
        _pr(head),
        _config(),
        repo,
        token="token",
    )

    assert result["status"] == "pushed"
    assert result["memory_sidecar"]["written"] is True
    assert result["commit_plan"] == [{**_commit_plan()[0], "paths": ["docs/a.md", *MEMORY_PATHS]}]
    assert subprocess.check_output(["git", "rev-list", "--count", "HEAD"], cwd=repo, text=True).strip() == "2"
    committed_paths = subprocess.check_output(["git", "show", "--name-only", "--format=", result["commit_sha"]], cwd=repo, text=True).splitlines()
    assert "docs/a.md" in committed_paths
    assert set(MEMORY_PATHS).issubset(committed_paths)
    ignored = subprocess.run(["git", "check-ignore", "--no-index", MEMORY_PATHS[0]], cwd=repo, capture_output=True, text=True)
    assert ignored.returncode == 0
    ledger_text = (repo / MEMORY_PATHS[0]).read_text(encoding="utf-8")
    ledger = json.loads(ledger_text)
    assert fake_secret not in ledger_text
    assert "[REDACTED_SECRET]" in ledger_text
    assert ledger["entries"][-1]["trusted"] is True
    assert ledger["entries"][-1]["provenance"]["signature"]
    assert "Generated from `review-memory.v1` ledger" in (repo / MEMORY_PATHS[1]).read_text(encoding="utf-8")
    assert result["policy_report"]["commits"][0]["touched_files"] == ["docs/a.md"]
    assert result["policy_report"]["commits"][0]["trusted_memory_paths"] == sorted(MEMORY_PATHS)


def test_gitignored_memory_files_force_add_only_when_explicit(tmp_path: Path) -> None:
    repo, head = _init_repo(tmp_path)
    (repo / "docs/a.md").write_text("new\n", encoding="utf-8")
    memory_file = repo / MEMORY_PATHS[0]
    memory_file.parent.mkdir(parents=True)
    memory_file.write_text('{"schema_version":"review-memory.v1","scope":{"repository":"owner/repo","pr_number":7,"base_ref":"main"},"entries":[]}\n', encoding="utf-8")
    plan = [{"subject": "docs(a): update sidecar force add", "body": "Update docs and sidecar.", "paths": ["docs/a.md", MEMORY_PATHS[0]]}]

    with pytest.raises(ValidationError, match="requires trusted force-add"):
        create_commits_from_plan(repo, plan, "plan", head, {"commit_plan": plan})

    with pytest.raises(ValidationError, match="must accompany non-memory fix paths"):
        create_commits_from_plan(
            repo,
            [{"subject": "docs(a): reject memory only sidecar", "body": "Reject a memory-only sidecar.", "paths": [MEMORY_PATHS[0]]}],
            "plan",
            head,
            {"commit_plan": plan},
            trusted_force_add_paths=[MEMORY_PATHS[0]],
            trusted_force_add_prefix=".omo/review-memory/",
        )

    with pytest.raises(ValidationError, match="not a canonical generated memory file"):
        nested_path = ".omo/review-memory/pr-7/nested/ledger.json"
        nested_file = repo / nested_path
        nested_file.parent.mkdir(parents=True, exist_ok=True)
        nested_file.write_text("{}\n", encoding="utf-8")
        create_commits_from_plan(
            repo,
            [{"subject": "docs(a): reject nested sidecar path", "body": "Reject nested memory path.", "paths": ["docs/a.md", nested_path]}],
            "plan",
            head,
            {"commit_plan": plan},
            trusted_force_add_paths=[nested_path],
            trusted_force_add_prefix=".omo/review-memory/",
        )

    shas = create_commits_from_plan(
        repo,
        plan,
        "plan",
        head,
        {"commit_plan": plan},
        trusted_force_add_paths=[MEMORY_PATHS[0]],
        trusted_force_add_prefix=".omo/review-memory/",
    )

    assert shas
    committed_paths = subprocess.check_output(["git", "show", "--name-only", "--format=", shas[-1]], cwd=repo, text=True).splitlines()
    assert "docs/a.md" in committed_paths
    assert MEMORY_PATHS[0] in committed_paths
    assert subprocess.run(["git", "check-ignore", "--no-index", MEMORY_PATHS[0]], cwd=repo, capture_output=True, text=True).returncode == 0


def test_writer_accepts_trusted_artifact_summaries(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "task-16-test-key")
    repo, head = _init_repo(tmp_path)
    fake_secret = "github_pat_" + "B" * 82

    report = write_trusted_push_memory_sidecar(
        repo,
        _pr(head),
        {"schema_version": "fix-merge-merged-fix.v1", "status": "ready_to_push", "patch": PATCH, "expected_head_sha": head, "round": 4},
        _validation(PATCH, "b" * 64),
        _commit_plan(),
        _config(),
        artifact_summaries=[
            {
                "kind": "decision",
                "category": "decisions",
                "source_stage": "techlead",
                "body": {"summary": f"Keep the trusted sidecar model and redact {fake_secret}."},
            }
        ],
    )

    ledger_text = (repo / MEMORY_PATHS[0]).read_text(encoding="utf-8")
    ledger = json.loads(ledger_text)
    assert report["written"] is True
    assert len(report["entries"]) == 2
    assert [entry["kind"] for entry in ledger["entries"]] == ["fix_applied", "decision"]
    assert ledger["entries"][1]["trusted"] is True
    assert fake_secret not in ledger_text
    assert "[REDACTED_SECRET]" in ledger_text
    assert "artifact-techlead-decision" in ledger["entries"][1]["entry_id"]
    assert "Keep the trusted sidecar model" in (repo / MEMORY_PATHS[2]).read_text(encoding="utf-8")


def test_unrelated_leftover_changes_still_fail_with_trusted_sidecar(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "task-16-test-key")
    repo, head = _init_repo(tmp_path)
    applied_hash = _preapply_patch(repo)
    (repo / "UNRELATED.txt").write_text("leftover\n", encoding="utf-8")
    _stub_push(monkeypatch)

    with pytest.raises(ValidationError, match="commit plan did not commit all applied changes"):
        commit_and_push_validated_fix(
            {"schema_version": "fix-merge-merged-fix.v1", "status": "ready_to_push", "patch": PATCH, "expected_head_sha": head},
            _validation(PATCH, applied_hash),
            _pr(head),
            _config(),
            repo,
            token="token",
        )


def test_writer_rejects_symlinked_memory_scope_before_write(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv(HMAC_KEY_ENV, "task-16-test-key")
    repo, head = _init_repo(tmp_path)
    outside = tmp_path / "outside"
    outside.mkdir()
    memory_root = repo / ".omo" / "review-memory"
    memory_root.mkdir(parents=True)
    (memory_root / "pr-7").symlink_to(outside, target_is_directory=True)

    with pytest.raises(ValidationError, match="unsafe memory output path"):
        write_trusted_push_memory_sidecar(
            repo,
            _pr(head),
            {"schema_version": "fix-merge-merged-fix.v1", "status": "ready_to_push", "patch": PATCH, "expected_head_sha": head},
            _validation(PATCH, "c" * 64),
            _commit_plan(),
            _config(),
        )

    assert not (outside / "ledger.json").exists()


def test_unvalidated_or_model_untrusted_artifacts_do_not_generate_sidecar(tmp_path: Path) -> None:
    config = _config()
    merged = {"schema_version": "fix-merge-merged-fix.v1", "status": "ready_to_push", "patch": PATCH, "expected_head_sha": "h"}
    unvalidated = {"schema_version": "push-validated-fix.v1", "status": "tests_failed", "validated": False}
    model_untrusted = {
        **_validation(PATCH, "a" * 64),
        "semantic_safety_approved": False,
        "semantic_safety": {"status": "rejected", "approved": False, "commit_plan": _commit_plan()},
    }

    first = write_trusted_push_memory_sidecar(tmp_path, _pr("h"), merged, unvalidated, _commit_plan(), config)
    second = write_trusted_push_memory_sidecar(tmp_path, _pr("h"), merged, model_untrusted, _commit_plan(), config)

    assert first["written"] is False
    assert first["reason"] == "unvalidated_push_artifact"
    assert second["written"] is False
    assert second["reason"] == "semantic_safety_not_trusted"
    assert not (tmp_path / ".omo").exists()
