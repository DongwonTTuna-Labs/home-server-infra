"""Commit helper for trusted push."""
from __future__ import annotations

import subprocess
from copy import deepcopy
from pathlib import Path
from typing import Any, Iterable, Sequence

from codex_review.core.errors import ValidationError
from codex_review.memory.paths import LEDGER_FILENAME, is_memory_path, is_safe_memory_path
from codex_review.memory.types import CATEGORY_NOTEPAD_FILES
from codex_review.patches.commit_plan import normalize_commit_plan
from codex_review.security.patch_policy import validate_patch_policy
from .safe_subprocess import sanitized_env

_GENERATED_MEMORY_FILENAMES = frozenset({LEDGER_FILENAME, *CATEGORY_NOTEPAD_FILES.values()})


def commit_plan_from_artifacts(merged_fix: dict[str, Any], validation_result: dict[str, Any] | None = None, patch_text: str | None = None) -> list[dict[str, Any]]:
    semantic = (validation_result or {}).get("semantic_safety")
    raw_plan = None
    if isinstance(semantic, dict):
        raw_plan = semantic.get("commit_plan")
    if raw_plan is None:
        raw_plan = merged_fix.get("commit_plan")
    return normalize_commit_plan(raw_plan, patch_text)


def attach_trusted_sidecar_to_commit_plan(commit_plan: Sequence[dict[str, Any]], sidecar_paths: Sequence[str]) -> list[dict[str, Any]]:
    """Return a commit plan whose final fix commit also carries trusted sidecar paths."""
    plan = [deepcopy(dict(entry)) for entry in commit_plan]
    if not sidecar_paths:
        return plan
    if not plan:
        raise ValidationError("trusted memory sidecar requires a non-empty commit plan")
    final_paths = list(plan[-1].get("paths") or [])
    seen = set(final_paths)
    for path in sidecar_paths:
        normalized = str(path)
        if normalized not in seen:
            final_paths.append(normalized)
            seen.add(normalized)
    plan[-1]["paths"] = final_paths
    return plan


def build_commit_message(merged_fix: dict[str, Any], design_plan_hash: str, old_head_sha: str, entry: dict[str, Any] | None = None) -> str:
    plan = normalize_commit_plan([entry] if entry is not None else merged_fix.get("commit_plan"))
    selected = plan[0]
    body = str(selected.get("body") or "").strip()
    parts = [selected["subject"], ""]
    if body:
        parts.extend([body, ""])
    parts.extend(
        [
            f"Design-plan-hash: {design_plan_hash}",
            f"Previous-head-sha: {old_head_sha}",
            "Marker: codex-review:autofix",
            "",
        ]
    )
    return "\n".join(parts)


def configure_git_author(policy: dict[str, Any]) -> None:
    name = policy.get("git_author_name", "Codex Review Bot")
    email = policy.get("git_author_email", "codex-review@example.invalid")
    subprocess.run(["git", "config", "user.name", name], check=True, env=sanitized_env())
    subprocess.run(["git", "config", "user.email", email], check=True, env=sanitized_env())


def create_commit(
    repo_path: str | Path,
    message: str,
    paths: list[str] | None = None,
    *,
    trusted_force_add_paths: Iterable[str] | None = None,
    trusted_force_add_prefix: str | None = None,
) -> str:
    repo = Path(repo_path)
    force_paths = _validated_trusted_force_add_paths(repo, trusted_force_add_paths or [], trusted_force_add_prefix)
    if paths:
        _reject_untrusted_memory_paths(paths, force_paths)
        normal_paths = [path for path in paths if path not in force_paths]
        forced_paths = [path for path in paths if path in force_paths]
        if forced_paths and not normal_paths:
            raise ValidationError("trusted memory sidecar paths must accompany non-memory fix paths")
        if normal_paths:
            subprocess.run(["git", "add", "--", *normal_paths], cwd=repo, check=True, env=sanitized_env())
        if forced_paths:
            subprocess.run(["git", "add", "-f", "--", *forced_paths], cwd=repo, check=True, env=sanitized_env())
    else:
        subprocess.run(["git", "add", "-A"], cwd=repo, check=True, env=sanitized_env())
    diff_proc = subprocess.run(["git", "diff", "--cached", "--quiet", "--"], cwd=repo, env=sanitized_env())
    if diff_proc.returncode == 0:
        raise ValidationError("commit plan entry produced no staged diff")
    proc = subprocess.run(["git", "commit", "--no-verify", "-F", "-"], input=message, cwd=repo, capture_output=True, text=True, env=sanitized_env())
    if proc.returncode != 0:
        raise ValidationError(f"git commit failed: {proc.stderr.strip()}")
    sha = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True, env=sanitized_env()).strip()
    return sha


def create_commits_from_plan(
    repo_path: str | Path,
    commit_plan: list[dict[str, Any]],
    design_plan_hash: str,
    old_head_sha: str,
    merged_fix: dict[str, Any],
    *,
    trusted_force_add_paths: Sequence[str] | None = None,
    trusted_force_add_prefix: str | None = None,
) -> list[str]:
    repo = Path(repo_path)
    force_paths = _validated_trusted_force_add_paths(repo, trusted_force_add_paths or [], trusted_force_add_prefix)
    subprocess.run(["git", "reset"], cwd=repo, check=True, stdout=subprocess.DEVNULL, env=sanitized_env())
    shas: list[str] = []
    for entry in commit_plan:
        message = build_commit_message(merged_fix, design_plan_hash, old_head_sha, entry)
        entry_paths = list(entry.get("paths") or [])
        shas.append(
            create_commit(
                repo,
                message,
                entry_paths,
                trusted_force_add_paths=[path for path in entry_paths if path in force_paths],
                trusted_force_add_prefix=trusted_force_add_prefix,
            )
        )
    status = subprocess.run(["git", "status", "--porcelain"], cwd=repo, capture_output=True, text=True, env=sanitized_env())
    if status.stdout.strip():
        raise ValidationError("commit plan did not commit all applied changes")
    return shas


def validate_commit_diff(
    repo_path: str | Path,
    commit_sha: str,
    policy: dict[str, Any],
    *,
    trusted_memory_paths: Sequence[str] | None = None,
    trusted_memory_prefix: str | None = None,
) -> dict[str, Any]:
    repo = Path(repo_path)
    patch = subprocess.check_output(["git", "show", "--format=", "--binary", commit_sha], cwd=repo, text=True, env=sanitized_env())
    force_paths = _validated_trusted_force_add_paths(repo, trusted_memory_paths or [], trusted_memory_prefix)
    reviewed_patch = _strip_trusted_memory_diff_sections(patch, force_paths)
    report = validate_patch_policy(reviewed_patch, policy, {})
    if force_paths:
        report["trusted_memory_paths"] = sorted(force_paths)
    return report


def _validated_trusted_force_add_paths(repo: Path, paths: Iterable[str], trusted_prefix: str | None) -> set[str]:
    out: set[str] = set()
    for raw_path in paths:
        path = str(raw_path).strip()
        if not path:
            continue
        if not trusted_prefix:
            raise ValidationError(f"trusted force-add path requires memory_write_prefix: {path}")
        prefix = _normalized_prefix(trusted_prefix)
        if not path.startswith(prefix):
            raise ValidationError(f"trusted force-add path is outside memory_write_prefix: {path}")
        if Path(path).name not in _GENERATED_MEMORY_FILENAMES:
            raise ValidationError(f"trusted force-add path is not a generated memory file: {path}")
        if not _is_canonical_generated_memory_path(path):
            raise ValidationError(f"trusted force-add path is not a canonical generated memory file: {path}")
        if not is_safe_memory_path(path, repo):
            raise ValidationError(f"trusted force-add path is not a safe memory path: {path}")
        full_path = repo / path
        if not full_path.is_file() or full_path.is_symlink():
            raise ValidationError(f"trusted force-add path does not name a regular generated file: {path}")
        out.add(path)
    return out


def _is_canonical_generated_memory_path(path: str) -> bool:
    parts = path.split("/")
    if len(parts) != 4:
        return False
    if parts[0] != ".omo" or parts[1] != "review-memory":
        return False
    scope = parts[2]
    if not scope.startswith("pr-"):
        return False
    number = scope[3:]
    if not number.isdigit() or number.startswith("0"):
        return False
    return int(number) > 0 and parts[3] in _GENERATED_MEMORY_FILENAMES


def _normalized_prefix(prefix: str) -> str:
    normalized = str(prefix or "").strip()
    if not normalized or normalized.startswith("/") or ".." in normalized.split("/") or "\\" in normalized:
        raise ValidationError("autofix.memory_write_prefix must be a relative safe prefix")
    return normalized if normalized.endswith("/") else f"{normalized}/"


def _reject_untrusted_memory_paths(paths: Sequence[str], trusted_force_paths: set[str]) -> None:
    for raw_path in paths:
        path = str(raw_path)
        if is_memory_path(path) and path not in trusted_force_paths:
            raise ValidationError(f"review memory path requires trusted force-add sidecar inclusion: {path}")


def _strip_trusted_memory_diff_sections(patch_text: str, trusted_memory_paths: set[str]) -> str:
    if not trusted_memory_paths:
        return patch_text
    kept: list[str] = []
    current: list[str] = []
    for line in patch_text.splitlines():
        if line.startswith("diff --git ") and current:
            if not _section_is_trusted_memory(current, trusted_memory_paths):
                kept.extend(current)
            current = []
        current.append(line)
    if current and not _section_is_trusted_memory(current, trusted_memory_paths):
        kept.extend(current)
    return "\n".join(kept) + ("\n" if kept else "")


def _section_is_trusted_memory(section: Sequence[str], trusted_memory_paths: set[str]) -> bool:
    header = next((line for line in section if line.startswith("diff --git ")), "")
    if not header:
        return False
    parts = header.split()
    candidates = [_strip_diff_prefix(part) for part in parts[2:4]] if len(parts) >= 4 else []
    return any(path in trusted_memory_paths for path in candidates)


def _strip_diff_prefix(path: str) -> str:
    return path[2:] if path.startswith(("a/", "b/")) else path
