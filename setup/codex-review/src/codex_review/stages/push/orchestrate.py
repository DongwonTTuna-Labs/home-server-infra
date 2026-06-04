"""Trusted push push orchestration.

Stage07 is deliberately split into two phases:

* validate/test: runs without any write token in the PR-head worktree, applies the
  patch and executes only trusted allowlisted tests with a sanitized env.
* commit/push: runs with the GitHub App installation token, re-checks out the
  exact PR head, reapplies the already validated patch, commits with hooks
  disabled, and pushes. It never runs tests.
"""
from __future__ import annotations

import hashlib
import subprocess
import tempfile
from pathlib import Path
from typing import Any

from codex_review.core.errors import CodexReviewError, ValidationError
from codex_review.github.app_token import assert_installation_token_for_repo, permissions_for_write_mode
from codex_review.security.patch_policy import validate_patch_policy
from .apply_patch import apply_merged_patch, collect_applied_diff, run_diff_check
from .commit import build_commit_message, commit_plan_from_artifacts, create_commit, create_commits_from_plan, validate_commit_diff
from .push import push_commit, verify_pushed_head
from .run_tests import run_required_tests, select_test_commands
from .validate import validate_current_head, validate_push_target, validate_ready_to_push, validate_worktree_clean
from .safe_subprocess import sanitized_env


def _policy(config: dict[str, Any], merged_fix: dict[str, Any]) -> dict[str, Any]:
    policy = dict(config.get("autofix", {}) or {})
    if isinstance(merged_fix.get("fix_policy"), dict):
        policy.update(merged_fix["fix_policy"])
    if isinstance(merged_fix.get("policy"), dict):
        policy.update(merged_fix["policy"])
    return policy


def _patch_text(merged_fix: dict[str, Any]) -> str:
    if merged_fix.get("patch_path"):
        return Path(merged_fix["patch_path"]).read_text(encoding="utf-8")
    return merged_fix.get("patch") or merged_fix.get("patch_text") or ""


def _sha256_text(text: str) -> str:
    return hashlib.sha256((text or "").encode("utf-8")).hexdigest()


def _write_temp_patch(patch_text: str) -> Path:
    handle = tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".patch", delete=False)
    with handle:
        handle.write(patch_text)
    return Path(handle.name)


def _git(repo_path: str | Path, *args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(["git", *args], cwd=Path(repo_path), capture_output=True, text=True, env=sanitized_env())
    if check and proc.returncode != 0:
        raise ValidationError(f"git {' '.join(args)} failed: {proc.stderr.strip()}")
    return proc


def _patch_already_applied(patch_text: str, repo_path: str | Path) -> bool:
    proc = subprocess.run(["git", "apply", "--reverse", "--check", "-"], input=patch_text, text=True, cwd=Path(repo_path), capture_output=True, env=sanitized_env())
    return proc.returncode == 0


def _terminal_nonpush_result(
    merged_fix: dict[str, Any],
    patch: str,
    head: str | None,
    status: str,
    reason: str,
    *,
    schema_version: str = "push-validated-fix.v1",
    error: BaseException | None = None,
) -> dict[str, Any]:
    result: dict[str, Any] = {
        "schema_version": schema_version,
        "status": status,
        "validated": False,
        "pushed": False,
        "commit_sha": None,
        "head_sha": head,
        "patch_hash": _sha256_text(patch),
        "reason": reason,
        "plan_hash": merged_fix.get("plan_hash") or merged_fix.get("design_plan_hash"),
    }
    if error is not None:
        result["error_type"] = error.__class__.__name__
    return result


def _no_fix_changes_result(merged_fix: dict[str, Any], patch: str, head: str | None, reason: str, *, schema_version: str = "push-validated-fix.v1") -> dict[str, Any]:
    return _terminal_nonpush_result(merged_fix, patch, head, "no_fix_changes", reason, schema_version=schema_version)


def _validation_failed_result(merged_fix: dict[str, Any], patch: str, head: str | None, error: BaseException, *, schema_version: str = "push-validated-fix.v1") -> dict[str, Any]:
    return _terminal_nonpush_result(merged_fix, patch, head, "validation_failed", str(error), schema_version=schema_version, error=error)


def _semantic_safety_status(semantic_safety: dict[str, Any] | None) -> str:
    if not semantic_safety:
        return "missing"
    return str(semantic_safety.get("status") or "unknown")


def _semantic_safety_gate_result(
    merged_fix: dict[str, Any],
    patch: str,
    head: str | None,
    semantic_safety: dict[str, Any] | None,
) -> dict[str, Any] | None:
    """Require AI semantic approval for the exact merged patch hash before push validation succeeds."""
    if not patch:
        return None
    expected_hash = _sha256_text(patch)
    if not semantic_safety:
        return _terminal_nonpush_result(
            merged_fix,
            patch,
            head,
            "semantic_safety_missing",
            "fix_merge semantic patch safety approval artifact is missing",
        )
    reviewed_hash = semantic_safety.get("patch_hash")
    if reviewed_hash != expected_hash:
        return _terminal_nonpush_result(
            merged_fix,
            patch,
            head,
            "semantic_safety_hash_mismatch",
            f"fix_merge semantic patch safety reviewed {reviewed_hash}, expected {expected_hash}",
        )
    if semantic_safety.get("status") != "approved" or semantic_safety.get("approved") is not True:
        reason = str(semantic_safety.get("blocking_reason") or semantic_safety.get("summary") or "fix_merge semantic patch safety did not approve the patch")
        return _terminal_nonpush_result(merged_fix, patch, head, "semantic_safety_rejected", reason)
    return None


def _configure_git_author(repo_path: str | Path, policy: dict[str, Any]) -> None:
    name = policy.get("git_author_name", "Codex Review Bot")
    email = policy.get("git_author_email", "codex-review@example.invalid")
    _git(repo_path, "config", "user.name", str(name))
    _git(repo_path, "config", "user.email", str(email))


def _current_head(repo_path: str | Path) -> str:
    return _git(repo_path, "rev-parse", "HEAD").stdout.strip()


def _validate_local_expected_head(repo_path: str | Path, pr_context: dict[str, Any], merged_fix: dict[str, Any]) -> str:
    expected = str(merged_fix.get("expected_head_sha") or pr_context.get("head_sha") or "")
    current = _current_head(repo_path)
    if expected and current != expected:
        raise ValidationError(f"local PR-head checkout does not match expected head: expected {expected}, current {current}")
    return current


def _ready_or_noop(merged_fix: dict[str, Any], patch: str) -> dict[str, Any] | None:
    status = merged_fix.get("status")
    if status in {"no_fix", "blocked"}:
        return {
            "schema_version": "push-result.v1",
            "status": status,
            "pushed": False,
            "commit_sha": None,
            "reason": "no merged patch is ready to push",
        }
    if not patch:
        return {
            "schema_version": "push-result.v1",
            "status": "empty_patch",
            "pushed": False,
            "commit_sha": None,
            "reason": "merged fix did not contain a patch",
        }
    return None


def validate_and_test_fix(
    merged_fix: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path,
    *,
    dry_run: bool = False,
    semantic_safety: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Apply and test a patch without any write token.

    The caller must run this in a PR-head checkout with no persisted credentials.
    Validation-stage failures are returned as structured ``validation_failed``
    artifacts instead of raising, so the workflow can continue into the
    duplicate issue / fallback stage instead of silently skipping it.
    """
    patch = ""
    head: str | None = None
    try:
        patch = _patch_text(merged_fix)
        noop = _ready_or_noop(merged_fix, patch)
        if noop:
            return {**noop, "schema_version": "push-validated-fix.v1", "validated": False}

        validate_ready_to_push({**merged_fix, "patch": patch})
        head = _validate_local_expected_head(repo_path, pr_context, merged_fix)
        policy = _policy(config, merged_fix)
        policy_report = validate_patch_policy(patch, policy, {})
        tests = select_test_commands(merged_fix, config)

        if dry_run:
            return {
                "schema_version": "push-validated-fix.v1",
                "status": "dry_run",
                "validated": False,
                "pushed": False,
                "commit_sha": None,
                "head_sha": head,
                "patch_hash": _sha256_text(patch),
                "policy_report": policy_report,
                "tests": tests,
            }

        validate_worktree_clean(repo_path)
        patch_path = _write_temp_patch(patch)
        try:
            try:
                apply_report = apply_merged_patch(patch_path, repo_path)
            except ValidationError as exc:
                if _patch_already_applied(patch, repo_path):
                    return _no_fix_changes_result(merged_fix, patch, head, f"patch is already present on the PR head: {exc}")
                raise
            try:
                run_diff_check(repo_path)
            except ValidationError as exc:
                return _no_fix_changes_result(merged_fix, patch, head, str(exc))
            applied_diff = collect_applied_diff(repo_path)
            applied_policy_report = validate_patch_policy(applied_diff, policy, {})
            semantic_block = _semantic_safety_gate_result(merged_fix, patch, head, semantic_safety)
            if semantic_block:
                return {
                    **semantic_block,
                    "applied_diff_hash": _sha256_text(applied_diff),
                    "apply_report": apply_report,
                    "policy_report": applied_policy_report,
                    "semantic_safety": semantic_safety or {},
                    "semantic_safety_status": _semantic_safety_status(semantic_safety),
                    "semantic_safety_approved": False,
                }
            test_report = run_required_tests(tests, repo_path)
            status = "validated" if test_report.get("passed", True) else "tests_failed"
            return {
                "schema_version": "push-validated-fix.v1",
                "status": status,
                "validated": status == "validated",
                "pushed": False,
                "commit_sha": None,
                "head_sha": head,
                "patch_hash": _sha256_text(patch),
                "applied_diff_hash": _sha256_text(applied_diff),
                "apply_report": apply_report,
                "policy_report": applied_policy_report,
                "test_report": test_report,
                "semantic_safety": semantic_safety or {},
                "semantic_safety_status": _semantic_safety_status(semantic_safety),
                "semantic_safety_approved": True,
            }
        finally:
            try:
                patch_path.unlink(missing_ok=True)
            except Exception:
                pass
    except (CodexReviewError, OSError, subprocess.SubprocessError) as exc:
        return _validation_failed_result(merged_fix, patch, head, exc)


def commit_validated_fix(merged_fix: dict[str, Any], pr_context: dict[str, Any], config: dict[str, Any], repo_path: str | Path) -> dict[str, Any]:
    """Commit an already-applied and validated fix in the trusted checkout."""
    policy = _policy(config, merged_fix)
    old_head = pr_context.get("head_sha") or merged_fix.get("expected_head_sha") or _current_head(repo_path)
    _configure_git_author(repo_path, policy)
    applied_diff = collect_applied_diff(repo_path)
    commit_plan = commit_plan_from_artifacts(merged_fix, {"semantic_safety": merged_fix.get("semantic_safety") or {}}, applied_diff)
    commit_shas = create_commits_from_plan(repo_path, commit_plan, str(merged_fix.get("plan_hash") or merged_fix.get("design_plan_hash") or "unknown"), str(old_head), merged_fix)
    reports = [validate_commit_diff(repo_path, sha, policy) for sha in commit_shas]
    return {"schema_version": "push-result.v1", "status": "committed", "pushed": False, "commit_sha": commit_shas[-1], "commit_shas": commit_shas, "commit_plan": commit_plan, "policy_report": {"commits": reports}}


def commit_and_push_validated_fix(
    merged_fix: dict[str, Any],
    validation_result: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path,
    token: str | None,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    """Reapply a validated patch, commit it, and push with an App token.

    This phase intentionally does not execute tests. It verifies that the diff
    produced by applying the patch matches the no-token validation phase.
    """
    patch = _patch_text(merged_fix)
    noop = _ready_or_noop(merged_fix, patch)
    if noop:
        return noop

    if not validation_result.get("validated"):
        status = validation_result.get("status") or "validation_failed"
        return {
            "schema_version": "push-result.v1",
            "status": status,
            "pushed": False,
            "commit_sha": None,
            "reason": "push commit/push skipped because no-token validation did not pass",
            "validation_result": validation_result,
        }
    if validation_result.get("semantic_safety_approved") is not True:
        status = "semantic_safety_missing" if not validation_result.get("semantic_safety") else "semantic_safety_rejected"
        return {
            "schema_version": "push-result.v1",
            "status": status,
            "pushed": False,
            "commit_sha": None,
            "reason": "push commit/push skipped because exact patch hash lacks AI semantic safety approval",
            "validation_result": validation_result,
        }
    if validation_result.get("patch_hash") and validation_result["patch_hash"] != _sha256_text(patch):
        raise ValidationError("merged patch changed after no-token validation")

    validate_push_target(pr_context)
    owner = str(pr_context.get("owner"))
    repo = str(pr_context.get("repo"))
    pr_number = int(pr_context.get("pr_number"))
    if not token:
        raise ValidationError("push actual push requires a GitHub App installation token")
    assert_installation_token_for_repo(token, owner, repo, permissions_for_write_mode("push"))
    try:
        validate_current_head(pr_context, merged_fix, token)
    except ValidationError as exc:
        if "head SHA drift" in str(exc):
            return _terminal_nonpush_result(merged_fix, patch, pr_context.get("head_sha"), "stale_head", str(exc), schema_version="push-result.v1")
        raise
    _validate_local_expected_head(repo_path, pr_context, merged_fix)

    policy = _policy(config, merged_fix)
    validate_worktree_clean(repo_path)
    validate_patch_policy(patch, policy, {})

    if dry_run:
        return {
            "schema_version": "push-result.v1",
            "status": "dry_run_commit_push",
            "pushed": False,
            "commit_sha": None,
            "patch_hash": _sha256_text(patch),
        }

    patch_path = _write_temp_patch(patch)
    old_head = pr_context.get("head_sha") or merged_fix.get("expected_head_sha") or _current_head(repo_path)
    try:
        try:
            apply_report = apply_merged_patch(patch_path, repo_path)
        except ValidationError as exc:
            if _patch_already_applied(patch, repo_path):
                return {**_no_fix_changes_result(merged_fix, patch, old_head, f"patch is already present on the PR head: {exc}", schema_version="push-result.v1"), "validation_result": validation_result}
            raise
        try:
            run_diff_check(repo_path)
        except ValidationError as exc:
            return {**_no_fix_changes_result(merged_fix, patch, old_head, str(exc), schema_version="push-result.v1"), "validation_result": validation_result}
        applied_diff = collect_applied_diff(repo_path)
        applied_hash = _sha256_text(applied_diff)
        expected_applied_hash = validation_result.get("applied_diff_hash")
        if expected_applied_hash and expected_applied_hash != applied_hash:
            raise ValidationError("applied diff differs from no-token validation artifact")
        applied_policy_report = validate_patch_policy(applied_diff, policy, {})
        _configure_git_author(repo_path, policy)
        design_plan_hash = str(merged_fix.get("plan_hash") or merged_fix.get("design_plan_hash") or "unknown")
        commit_plan = commit_plan_from_artifacts(merged_fix, validation_result, applied_diff)
        commit_shas = create_commits_from_plan(repo_path, commit_plan, design_plan_hash, str(old_head), merged_fix)
        commit_sha = commit_shas[-1]
        commit_policy_reports = [validate_commit_diff(repo_path, sha, policy) for sha in commit_shas]
        commit_policy_report = {"commits": commit_policy_reports, "passed": all(r.get("passed", True) for r in commit_policy_reports)}
        head_ref = pr_context.get("head_ref")
        if not head_ref:
            return {"schema_version": "push-result.v1", "status": "committed_no_head_ref", "pushed": False, "commit_sha": commit_sha, "commit_shas": commit_shas, "commit_plan": commit_plan, "policy_report": commit_policy_report}
        push_report = push_commit(repo_path, str(head_ref), owner, repo, token)
        pushed = bool(push_report.get("pushed"))
        verified = bool(push_report.get("verified"))
        if pushed and not verified:
            verified = verify_pushed_head(owner, repo, pr_number, commit_sha, token)
        # Security gate: export updated_head_sha only when the push is remote-verified
        # (ls-remote or PR head API confirmed remote == commit_sha); never on a local-only SHA.
        remote_head_sha = str(push_report.get("remote_head_sha") or "")
        updated_head_sha = commit_sha if (pushed and verified) else ""
        return {
            "schema_version": "push-result.v1",
            "status": "pushed" if pushed and verified else "pushed_unverified" if pushed else "push_failed",
            "pushed": pushed,
            "commit_sha": commit_sha,
            "commit_shas": commit_shas,
            "commit_plan": commit_plan,
            "verified": verified,
            "remote_head_sha": remote_head_sha,
            "updated_head_sha": updated_head_sha,
            "apply_report": apply_report,
            "policy_report": commit_policy_report,
            "applied_policy_report": applied_policy_report,
            "push_report": push_report,
            "validation_result": {"status": validation_result.get("status"), "applied_diff_hash": validation_result.get("applied_diff_hash")},
        }
    finally:
        try:
            patch_path.unlink(missing_ok=True)
        except Exception:
            pass


def run_push_flow(
    merged_fix: dict[str, Any],
    pr_context: dict[str, Any],
    config: dict[str, Any],
    repo_path: str | Path,
    token: str | None,
    *,
    dry_run: bool = False,
) -> dict[str, Any]:
    """Backward-compatible wrapper for older callers.

    In dry-run mode this only validates. In actual mode it still sanitizes the
    test environment, but production workflows should prefer the explicit
    validate/test + commit/push split.
    """
    if dry_run:
        validation = validate_and_test_fix(merged_fix, pr_context, config, repo_path, dry_run=True)
        status = validation.get("status") if validation.get("status") in {"no_fix", "blocked", "no_fix_changes", "empty_patch", "tests_failed", "validation_failed"} else "dry_run"
        return {**validation, "schema_version": "push-result.v1", "status": status, "pushed": False}
    validation = validate_and_test_fix(merged_fix, pr_context, config, repo_path, dry_run=False)
    return commit_and_push_validated_fix(merged_fix, validation, pr_context, config, repo_path, token, dry_run=False)
