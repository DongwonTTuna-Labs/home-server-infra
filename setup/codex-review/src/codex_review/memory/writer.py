"""Trusted push-boundary review-memory writer."""
from __future__ import annotations

import hashlib
import json
import subprocess
from copy import deepcopy
from datetime import UTC, datetime
from pathlib import Path
from typing import Any, Mapping, Sequence

from codex_review.core.config import DEFAULT_CONFIG
from codex_review.core.errors import CodexReviewError, ValidationError
from codex_review.memory.ledger import append_entries, read_ledger, write_ledger
from codex_review.memory.paths import is_memory_path, is_safe_memory_path, ledger_path, memory_scope_dir
from codex_review.memory.provenance import sign_entry
from codex_review.memory.redaction import redact_memory_text
from codex_review.memory.render import compact, write_projection_files
from codex_review.memory.types import CATEGORY_NOTEPAD_FILES, validate_review_memory_entry
from codex_review.security.redaction import safe_log_value
from codex_review.security.subprocess_env import sanitized_env
from codex_review.stages.push.push import push_commit
from codex_review.stages.push.validate import validate_push_target

MEMORY_COMMIT_TRAILER = "codex-memory: true"
MEMORY_COMMIT_SUBJECT = "chore(codex-memory): record terminal review memory"

_TERMINAL_STATUS_MAP = {
    "lgtm": "lgtm",
    "stop_lgtm": "lgtm",
    "no_fix": "no_fix_needed",
    "empty_patch": "no_fix_needed",
    "no_fix_needed": "no_fix_needed",
    "no_fix_changes": "no_fix_changes",
    "no_diff_repeat": "no_fix_changes",
    "no-diff-repeat": "no_fix_changes",
    "oscillation_detected": "no_fix_changes",
    "max_rounds_reached": "no_fix_changes",
}
_FORBIDDEN_SKIP_MARKERS = ("[skip ci]", "[ci skip]", "skip-checks:true", "skip-checks: true")


def write_trusted_push_memory_sidecar(
    repo_path: str | Path,
    pr_context: Mapping[str, Any],
    merged_fix: Mapping[str, Any],
    validation_result: Mapping[str, Any],
    commit_plan: Sequence[Mapping[str, Any]],
    config: Mapping[str, Any],
    *,
    artifact_summaries: Sequence[Mapping[str, Any]] | None = None,
    created_at: str | None = None,
) -> dict[str, Any]:
    """Append a trusted review-memory entry and render generated projections.

    The writer accepts only already-validated trusted push artifacts. Model output
    is summarized only after the no-token validation artifact proves the exact
    patch and semantic safety approval were validated by trusted code.
    """
    skip_reason = _skip_reason(merged_fix, validation_result, commit_plan, config)
    if skip_reason:
        return {"written": False, "reason": skip_reason, "paths": [], "entries": []}

    repo = Path(repo_path)
    scope = _scope(pr_context, config)
    created = created_at or _now_iso()
    ledger_rel = ledger_path(scope["repository"], scope["pr_number"])
    scope_rel = memory_scope_dir(scope["repository"], scope["pr_number"])
    paths = [ledger_rel.as_posix(), *[(scope_rel / filename).as_posix() for filename in CATEGORY_NOTEPAD_FILES.values()]]
    _validate_generated_output_paths(repo, paths)

    entries = [
        _signed_push_entry(
            scope=scope,
            pr_context=pr_context,
            merged_fix=merged_fix,
            validation_result=validation_result,
            commit_plan=commit_plan,
            created_at=created,
        ),
        *_signed_artifact_summary_entries(
            scope=scope,
            pr_context=pr_context,
            merged_fix=merged_fix,
            validation_result=validation_result,
            summaries=artifact_summaries or [],
            created_at=created,
        ),
    ]
    for entry in entries:
        validate_review_memory_entry(entry)

    ledger = read_ledger(repo, scope)
    max_entries = _positive_int(_memory_config(config).get("max_entries", 200), "memory.max_entries")
    updated = append_entries(ledger, entries, max_entries=max_entries)
    compacted, markdown_files = compact(updated, config)

    scope_dir = repo / scope_rel
    write_ledger(repo / ledger_rel, compacted)
    write_projection_files(scope_dir, markdown_files)

    return {
        "written": True,
        "reason": "trusted_push_artifacts",
        "scope": scope,
        "paths": paths,
        "entries": entries,
        "entry_ids": [entry["entry_id"] for entry in entries],
    }


def write_terminal_memory_commit(
    record: Mapping[str, Any],
    pr_context: Mapping[str, Any] | None,
    token: str | None,
    repo_path: str | Path = ".",
    config: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Best-effort terminal LGTM/no-fix memory-only commit and push."""
    try:
        return _write_terminal_memory_commit(record, pr_context or {}, token, repo_path, _terminal_config(config))
    except Exception as exc:
        return _terminal_failure_report(exc)


def terminal_memory_status(record: Mapping[str, Any]) -> str:
    if bool(record.get("pushed")):
        return ""
    if bool(record.get("lgtm")):
        return "lgtm"
    for value in _terminal_candidates(record):
        normalized = _TERMINAL_STATUS_MAP.get(str(value or "").strip())
        if normalized:
            return normalized
    return ""


def build_terminal_memory_entry(
    record: Mapping[str, Any],
    pr_context: Mapping[str, Any] | None,
    config: Mapping[str, Any] | None = None,
    *,
    created_at: str | None = None,
) -> dict[str, Any]:
    status = terminal_memory_status(record)
    if not status:
        raise ValidationError("record is not a terminal LGTM/no-fix memory outcome")
    cfg = _terminal_config(config)
    ctx = pr_context or {}
    body = _redact_json(
        {
            "summary": _terminal_summary(status),
            "terminal_status": status,
            "raw_outcome": _raw_terminal_outcome(record),
            "reason": _terminal_reason(record),
            "pushed": bool(record.get("pushed")),
            "audit": "memory-only terminal summary; not a routing or CI-suppression signal",
        }
    )
    entry = {
        "entry_id": _terminal_entry_id(record, ctx, cfg, status),
        "created_at": created_at or _now_iso(),
        "round": _terminal_round(record, ctx),
        "head_sha": _terminal_head_sha(record, ctx),
        "kind": "learning",
        "category": "learnings",
        "body": body,
        "source_stage": "reentry_terminal_memory",
        "trusted": False,
    }
    signed = sign_entry(entry)
    validate_review_memory_entry(signed)
    return signed


def build_memory_commit_message(entry: Mapping[str, Any]) -> str:
    message = "\n".join(
        [
            MEMORY_COMMIT_SUBJECT,
            "",
            f"Record terminal review memory entry {entry.get('entry_id', 'unknown')}.",
            "",
            MEMORY_COMMIT_TRAILER,
            "",
        ]
    )
    lowered = message.lower()
    if any(marker in lowered for marker in _FORBIDDEN_SKIP_MARKERS):
        raise ValidationError("memory commit message must not contain CI skip markers")
    return message


def _skip_reason(
    merged_fix: Mapping[str, Any],
    validation_result: Mapping[str, Any],
    commit_plan: Sequence[Mapping[str, Any]],
    config: Mapping[str, Any],
) -> str:
    memory = config.get("memory") if isinstance(config.get("memory"), Mapping) else None
    if not memory or memory.get("enabled") is not True:
        return "memory_disabled"
    if validation_result.get("validated") is not True or validation_result.get("status") != "validated":
        return "unvalidated_push_artifact"
    if validation_result.get("semantic_safety_approved") is not True:
        return "semantic_safety_not_trusted"
    semantic = validation_result.get("semantic_safety")
    if not isinstance(semantic, Mapping) or semantic.get("approved") is not True or semantic.get("status") != "approved":
        return "semantic_safety_not_trusted"
    patch = _patch_text(merged_fix)
    expected_patch_hash = str(validation_result.get("patch_hash") or "")
    if not patch or not expected_patch_hash or expected_patch_hash != _sha256_text(patch):
        return "patch_hash_not_validated"
    if not str(validation_result.get("applied_diff_hash") or ""):
        return "applied_diff_hash_missing"
    if not commit_plan:
        return "commit_plan_missing"
    return ""


def _signed_push_entry(
    *,
    scope: dict[str, Any],
    pr_context: Mapping[str, Any],
    merged_fix: Mapping[str, Any],
    validation_result: Mapping[str, Any],
    commit_plan: Sequence[Mapping[str, Any]],
    created_at: str,
) -> dict[str, Any]:
    head_sha = str(validation_result.get("head_sha") or pr_context.get("head_sha") or merged_fix.get("expected_head_sha") or "")
    round_number = _round_number(merged_fix, validation_result, pr_context)
    patch_hash = str(validation_result.get("patch_hash") or "")
    applied_diff_hash = str(validation_result.get("applied_diff_hash") or "")
    body = _redact_json(
        {
            "summary": f"Trusted push boundary validated and committed autofix sidecar for PR #{scope['pr_number']}.",
            "repository": scope["repository"],
            "pr_number": scope["pr_number"],
            "base_ref": scope["base_ref"],
            "head_ref": pr_context.get("head_ref") or "",
            "patch_hash": patch_hash,
            "applied_diff_hash": applied_diff_hash,
            "plan_hash": merged_fix.get("plan_hash") or merged_fix.get("design_plan_hash") or "",
            "validation_status": validation_result.get("status") or "",
            "semantic_safety_status": (validation_result.get("semantic_safety") or {}).get("status") if isinstance(validation_result.get("semantic_safety"), Mapping) else "",
            "test_report": _test_report_summary(validation_result.get("test_report")),
            "commit_plan": _commit_plan_summary(commit_plan),
        }
    )
    digest = _entry_digest(
        {
            "repository": scope["repository"],
            "pr_number": scope["pr_number"],
            "round": round_number,
            "head_sha": head_sha,
            "patch_hash": patch_hash,
            "applied_diff_hash": applied_diff_hash,
            "commit_plan": body["commit_plan"],
        }
    )
    entry = {
        "entry_id": f"push-fix-pr{scope['pr_number']}-r{round_number}-{digest[:12]}",
        "created_at": created_at,
        "round": round_number,
        "head_sha": head_sha,
        "kind": "fix_applied",
        "category": "learnings",
        "body": body,
        "source_stage": "push",
        "trusted": False,
    }
    return sign_entry(entry)


def _signed_artifact_summary_entries(
    *,
    scope: dict[str, Any],
    pr_context: Mapping[str, Any],
    merged_fix: Mapping[str, Any],
    validation_result: Mapping[str, Any],
    summaries: Sequence[Mapping[str, Any]],
    created_at: str,
) -> list[dict[str, Any]]:
    entries: list[dict[str, Any]] = []
    head_sha = str(validation_result.get("head_sha") or pr_context.get("head_sha") or merged_fix.get("expected_head_sha") or "")
    round_number = _round_number(merged_fix, validation_result, pr_context)
    for index, summary in enumerate(summaries, 1):
        if not isinstance(summary, Mapping):
            raise ValidationError("memory artifact summary must be an object")
        kind = str(summary.get("kind") or "").strip()
        if not kind:
            raise ValidationError("memory artifact summary kind is required")
        category = str(summary.get("category") or _category_for_kind(kind)).strip()
        source_stage = str(summary.get("source_stage") or summary.get("stage") or "push").strip()
        body_source = summary.get("body") if isinstance(summary.get("body"), Mapping) else {"summary": str(summary.get("summary") or "")}
        body = _redact_json(body_source)
        digest = _entry_digest(
            {
                "scope": scope,
                "index": index,
                "kind": kind,
                "category": category,
                "source_stage": source_stage,
                "body": body,
                "head_sha": head_sha,
                "round": round_number,
            }
        )
        entry = {
            "entry_id": str(summary.get("entry_id") or f"artifact-{source_stage}-{kind}-pr{scope['pr_number']}-r{round_number}-{digest[:12]}"),
            "created_at": str(summary.get("created_at") or created_at),
            "round": _non_negative_int(summary.get("round", round_number), "memory artifact summary round"),
            "head_sha": str(summary.get("head_sha") or head_sha),
            "kind": kind,
            "category": category,
            "body": body,
            "source_stage": source_stage,
            "trusted": False,
        }
        fingerprint = str(summary.get("finding_fingerprint") or "").strip()
        if fingerprint:
            entry["finding_fingerprint"] = redact_memory_text(fingerprint)
        signed = sign_entry(entry)
        validate_review_memory_entry(signed)
        entries.append(signed)
    return entries


def _category_for_kind(kind: str) -> str:
    normalized = str(kind or "").strip()
    if normalized in {"fix_applied", "learning", "resolved_finding"}:
        return "learnings"
    if normalized == "decision":
        return "decisions"
    if normalized == "open_risk":
        return "issues"
    if normalized == "rejected_approach":
        return "problems"
    return ""


def _validate_generated_output_paths(repo: Path, paths: Sequence[str]) -> None:
    for path in paths:
        if not is_safe_memory_path(path, repo):
            raise ValidationError(f"unsafe memory output path: {path}")


def _scope(pr_context: Mapping[str, Any], config: Mapping[str, Any]) -> dict[str, Any]:
    owner = str(pr_context.get("owner") or "").strip()
    repo = str(pr_context.get("repo") or "").strip()
    repository = str(pr_context.get("repository") or pr_context.get("base_repo_full_name") or "").strip()
    if not repository and owner and repo:
        repository = f"{owner}/{repo}"
    pr_number = _positive_int(pr_context.get("pr_number"), "pr_context.pr_number")
    base_ref = str(pr_context.get("base_ref") or config.get("base_branch") or "").strip()
    return {"repository": repository, "pr_number": pr_number, "base_ref": base_ref}


def _memory_config(config: Mapping[str, Any]) -> Mapping[str, Any]:
    memory = config.get("memory") if isinstance(config.get("memory"), Mapping) else {}
    return memory


def _patch_text(merged_fix: Mapping[str, Any]) -> str:
    if merged_fix.get("patch_path"):
        return Path(str(merged_fix["patch_path"])).read_text(encoding="utf-8")
    return str(merged_fix.get("patch") or merged_fix.get("patch_text") or "")


def _commit_plan_summary(commit_plan: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    summary: list[dict[str, Any]] = []
    for entry in commit_plan:
        item: dict[str, Any] = {
            "subject": str(entry.get("subject") or ""),
            "paths": [str(path) for path in entry.get("paths") or []],
        }
        body = str(entry.get("body") or "").strip()
        if body:
            item["body"] = body
        summary.append(item)
    return summary


def _test_report_summary(report: Any) -> dict[str, Any]:
    if not isinstance(report, Mapping):
        return {}
    out: dict[str, Any] = {}
    for key in ("passed", "status", "command_count", "failed", "skipped"):
        if key in report:
            out[key] = report[key]
    if "commands" in report and isinstance(report["commands"], list):
        out["commands"] = [
            {k: command[k] for k in ("id", "passed", "returncode") if isinstance(command, Mapping) and k in command}
            for command in report["commands"]
        ]
    return out


def _redact_json(value: Any) -> Any:
    if isinstance(value, str):
        return redact_memory_text(value)
    if isinstance(value, Mapping):
        return {str(key): _redact_json(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_redact_json(item) for item in value]
    if isinstance(value, tuple):
        return [_redact_json(item) for item in value]
    return deepcopy(value)


def _entry_digest(payload: Mapping[str, Any]) -> str:
    text = json.dumps(payload, sort_keys=True, separators=(",", ":"), default=str)
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _round_number(*sources: Mapping[str, Any]) -> int:
    for source in sources:
        for key in ("round", "round_number", "loop_round", "attempt", "attempt_number"):
            value = source.get(key)
            if value is not None and value != "":
                return _non_negative_int(value, key)
    return 1


def _positive_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValidationError(f"{name} must be a positive integer")
    try:
        normalized = int(value)
    except (TypeError, ValueError):
        raise ValidationError(f"{name} must be a positive integer") from None
    if normalized <= 0:
        raise ValidationError(f"{name} must be a positive integer")
    return normalized


def _non_negative_int(value: Any, name: str) -> int:
    if isinstance(value, bool):
        raise ValidationError(f"{name} must be a non-negative integer")
    try:
        normalized = int(value)
    except (TypeError, ValueError):
        raise ValidationError(f"{name} must be a non-negative integer") from None
    if normalized < 0:
        raise ValidationError(f"{name} must be a non-negative integer")
    return normalized


def _now_iso() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _write_terminal_memory_commit(
    record: Mapping[str, Any],
    pr_context: Mapping[str, Any],
    token: str | None,
    repo_path: str | Path,
    config: Mapping[str, Any],
) -> dict[str, Any]:
    if _memory_config(config).get("enabled") is not True:
        return _terminal_skip_report("memory_disabled")
    status = terminal_memory_status(record)
    if not status:
        return _terminal_skip_report("not_terminal_memory_outcome")
    if not token:
        return _terminal_skip_report("missing_write_token")
    try:
        validate_push_target(dict(pr_context))
    except CodexReviewError as exc:
        return _terminal_skip_report(f"unsafe_write_context: {safe_log_value(exc)}")

    repo = Path(repo_path)
    if _has_non_memory_worktree_changes(repo):
        return _terminal_skip_report("unsafe_write_context: non_memory_worktree_changes")

    scope = _scope(pr_context, config)
    entry = build_terminal_memory_entry(record, pr_context, config)
    ledger = read_ledger(repo, scope)
    max_entries = _positive_int(_memory_config(config).get("max_entries", 200), "memory.max_entries")
    updated = append_entries(ledger, [entry], max_entries=max_entries)
    compacted, markdown_files = compact(updated, config)
    paths = _write_terminal_memory_files(repo, scope, compacted, markdown_files)
    commit_sha = _create_terminal_memory_commit(repo, paths, build_memory_commit_message(entry), config)
    if not commit_sha:
        return {**_terminal_skip_report("no_memory_changes"), "entry_id": entry["entry_id"], "paths": paths}

    push_report = push_commit(
        repo,
        str(pr_context.get("head_ref") or ""),
        str(pr_context.get("owner") or ""),
        str(pr_context.get("repo") or ""),
        token,
    )
    pushed = bool(push_report.get("pushed"))
    return {
        "status": "pushed" if pushed else "push_failed",
        "memory_only": True,
        "pushed": pushed,
        "verified": bool(push_report.get("verified")),
        "commit_sha": commit_sha,
        "entry_id": entry["entry_id"],
        "paths": paths,
        "commit_message_trailer": MEMORY_COMMIT_TRAILER,
        "push_report": _redact_json(push_report),
    }


def _terminal_config(config: Mapping[str, Any] | None) -> dict[str, Any]:
    if not isinstance(config, Mapping):
        return deepcopy(DEFAULT_CONFIG)
    merged = deepcopy(DEFAULT_CONFIG)
    for key, value in config.items():
        if isinstance(value, Mapping) and isinstance(merged.get(key), Mapping):
            merged[key] = {**dict(merged[key]), **dict(value)}
        else:
            merged[key] = deepcopy(value)
    return merged


def _terminal_candidates(record: Mapping[str, Any]) -> list[Any]:
    push_result = _push_result(record)
    return [
        record.get("terminal_reason"),
        record.get("status"),
        record.get("route"),
        push_result.get("loop_terminal_reason"),
        push_result.get("terminal_reason"),
        push_result.get("status"),
        push_result.get("route"),
    ]


def _push_result(record: Mapping[str, Any]) -> dict[str, Any]:
    direct = record.get("push_result")
    if isinstance(direct, Mapping):
        return dict(direct)
    artifacts = record.get("artifacts")
    if isinstance(artifacts, Mapping) and isinstance(artifacts.get("push_result"), Mapping):
        return dict(artifacts["push_result"])
    return {}


def _raw_terminal_outcome(record: Mapping[str, Any]) -> str:
    if bool(record.get("lgtm")):
        return "lgtm"
    for value in _terminal_candidates(record):
        text = str(value or "").strip()
        if text:
            return redact_memory_text(text)
    return terminal_memory_status(record)


def _terminal_reason(record: Mapping[str, Any]) -> str:
    push_result = _push_result(record)
    for value in (
        record.get("reason"),
        record.get("persist_reason"),
        record.get("terminal_reason"),
        push_result.get("reason"),
        push_result.get("loop_budget_reason"),
        push_result.get("terminal_reason"),
    ):
        text = str(value or "").strip()
        if text:
            return redact_memory_text(text)
    return terminal_memory_status(record)


def _terminal_summary(status: str) -> str:
    if status == "lgtm":
        return "Terminal LGTM outcome recorded for review memory."
    if status == "no_fix_changes":
        return "Terminal no-fix-changes outcome recorded for review memory."
    return "Terminal no-fix-needed outcome recorded for review memory."


def _terminal_entry_id(record: Mapping[str, Any], pr_context: Mapping[str, Any], config: Mapping[str, Any], status: str) -> str:
    material = {
        "scope": _scope(pr_context, config),
        "status": status,
        "raw": _raw_terminal_outcome(record),
        "reason": _terminal_reason(record),
        "round": _terminal_round(record, pr_context),
        "head_sha": _terminal_head_sha(record, pr_context),
    }
    digest = _entry_digest(material)[:12]
    return f"terminal-{status.replace('_', '-')}-r{material['round']}-{digest}"


def _terminal_round(record: Mapping[str, Any], pr_context: Mapping[str, Any]) -> int:
    loop_state = record.get("loop_state") if isinstance(record.get("loop_state"), Mapping) else {}
    push_result = _push_result(record)
    for source in (loop_state, record, push_result, pr_context):
        if not isinstance(source, Mapping):
            continue
        for key in ("round_count", "round", "round_number", "iteration"):
            if key in source and source[key] not in {None, ""}:
                try:
                    return _non_negative_int(source[key], key)
                except ValidationError:
                    continue
    return 0


def _terminal_head_sha(record: Mapping[str, Any], pr_context: Mapping[str, Any]) -> str:
    push_result = _push_result(record)
    loop_state = record.get("loop_state") if isinstance(record.get("loop_state"), Mapping) else {}
    for value in (
        push_result.get("head_sha"),
        push_result.get("old_head"),
        record.get("head_sha"),
        record.get("commit_sha"),
        pr_context.get("head_sha"),
        loop_state.get("head_sha") if isinstance(loop_state, Mapping) else None,
    ):
        text = str(value or "").strip()
        if text:
            return redact_memory_text(text)
    return "unknown"


def _write_terminal_memory_files(repo: Path, scope: Mapping[str, Any], ledger: Mapping[str, Any], markdown_files: Mapping[str, str]) -> list[str]:
    ledger_rel = ledger_path(scope["repository"], scope["pr_number"])
    scope_rel = memory_scope_dir(scope["repository"], scope["pr_number"])
    paths = [ledger_rel.as_posix(), *[(scope_rel / filename).as_posix() for filename in CATEGORY_NOTEPAD_FILES.values()]]
    for path in paths:
        if not is_safe_memory_path(path, repo):
            raise ValidationError(f"unsafe memory output path: {path}")
    write_ledger(repo / ledger_rel, ledger)
    write_projection_files(repo / scope_rel, markdown_files)
    return paths


def _create_terminal_memory_commit(repo: Path, paths: list[str], message: str, config: Mapping[str, Any]) -> str:
    _configure_terminal_git_author(repo, config)
    _git(repo, "add", "-f", "--", *paths)
    diff = _git(repo, "diff", "--cached", "--quiet", "--", *paths, check=False)
    if diff.returncode == 0:
        return ""
    if diff.returncode != 1:
        raise ValidationError(f"git diff --cached failed: {safe_log_value(diff.stderr)}")
    _git(repo, "commit", "--no-verify", "-F", "-", "--", *paths, input_text=message)
    return _git(repo, "rev-parse", "HEAD").stdout.strip()


def _configure_terminal_git_author(repo: Path, config: Mapping[str, Any]) -> None:
    autofix = config.get("autofix") if isinstance(config.get("autofix"), Mapping) else {}
    name = str(autofix.get("git_author_name") or "Codex Review Bot")
    email = str(autofix.get("git_author_email") or "codex-review@example.invalid")
    _git(repo, "config", "user.name", name)
    _git(repo, "config", "user.email", email)


def _git(repo: Path, *args: str, check: bool = True, input_text: str | None = None) -> subprocess.CompletedProcess[str]:
    proc = subprocess.run(
        ["git", *args],
        cwd=repo,
        input=input_text,
        capture_output=True,
        text=True,
        env=sanitized_env(),
    )
    if check and proc.returncode != 0:
        raise ValidationError(f"git {' '.join(args)} failed: {safe_log_value(proc.stderr.strip())}")
    return proc


def _has_non_memory_worktree_changes(repo: Path) -> bool:
    for line in _git(repo, "status", "--porcelain", "--untracked-files=all").stdout.splitlines():
        for path in _porcelain_paths(line):
            if not is_memory_path(path):
                return True
    return False


def _porcelain_paths(line: str) -> list[str]:
    path = line[3:] if len(line) > 3 else line
    if " -> " in path:
        old_path, new_path = path.split(" -> ", 1)
        return [old_path, new_path]
    return [path]


def _terminal_skip_report(reason: str) -> dict[str, Any]:
    return {"status": "skipped", "reason": reason, "memory_only": True, "pushed": False}


def _terminal_failure_report(exc: BaseException) -> dict[str, Any]:
    return {
        "status": "failed",
        "reason": "terminal memory write failed",
        "non_fatal": True,
        "memory_only": True,
        "pushed": False,
        "error_type": exc.__class__.__name__,
        "error": safe_log_value(exc),
    }
