import subprocess

from codex_review.security import patch_policy
from codex_review.stages.fix_merge import premerge, validate as merge_validate
from codex_review.stages.push import validate as push_validate


class FakeProc:
    returncode = 0
    stdout = ""
    stderr = ""


def test_patch_policy_git_apply_check_strips_tokens(tmp_path, monkeypatch):
    (tmp_path / ".git").mkdir()
    monkeypatch.setenv("GITHUB_TOKEN", "secret")
    seen = {}
    def fake_run(*args, **kwargs):
        seen.update(kwargs.get("env") or {})
        return FakeProc()
    monkeypatch.setattr(subprocess, "run", fake_run)
    patch_policy.git_apply_check("", tmp_path)
    assert "GITHUB_TOKEN" not in seen


def test_fix_merge_premerge_git_commands_strip_tokens(tmp_path, monkeypatch):
    (tmp_path / ".git").mkdir()
    monkeypatch.setenv("GITHUB_TOKEN", "secret")
    envs = []
    def fake_run(*args, **kwargs):
        envs.append(kwargs.get("env") or {})
        return FakeProc()
    monkeypatch.setattr(subprocess, "run", fake_run)
    premerge.apply_patches_in_temp_worktree([""], tmp_path)
    assert envs
    assert all("GITHUB_TOKEN" not in env for env in envs)


def test_fix_merge_validate_git_apply_check_strips_tokens(tmp_path, monkeypatch):
    (tmp_path / ".git").mkdir()
    monkeypatch.setenv("GITHUB_TOKEN", "secret")
    seen = {}
    def fake_run(*args, **kwargs):
        seen.update(kwargs.get("env") or {})
        return FakeProc()
    monkeypatch.setattr(subprocess, "run", fake_run)
    merge_validate.validate_merged_patch_applies("", tmp_path)
    assert "GITHUB_TOKEN" not in seen


def test_push_worktree_clean_strips_tokens(tmp_path, monkeypatch):
    monkeypatch.setenv("GITHUB_TOKEN", "secret")
    seen = {}
    def fake_run(*args, **kwargs):
        seen.update(kwargs.get("env") or {})
        return FakeProc()
    monkeypatch.setattr(subprocess, "run", fake_run)
    push_validate.validate_worktree_clean(tmp_path)
    assert "GITHUB_TOKEN" not in seen
