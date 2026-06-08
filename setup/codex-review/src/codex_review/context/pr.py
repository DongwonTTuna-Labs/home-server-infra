"""PR context artifact builder."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.context.diff import build_changed_line_map, serialize_changed_line_map
from codex_review.context.diff import hunk_headers, summarize_diff
from codex_review.context.budget import estimate_tokens, tokens_to_chars
from codex_review.memory.paths import is_memory_path


def _changed_file_path(file_info: dict[str, Any]) -> str:
    return str(file_info.get("filename") or file_info.get("path") or file_info.get("new_path") or "")


def _review_code_files(files: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [file_info for file_info in files or [] if not is_memory_path(_changed_file_path(file_info))]


def _strip_diff_prefix(path: str) -> str:
    return path[2:] if path.startswith(("a/", "b/")) else path


def _diff_section_path(lines: list[str]) -> str:
    for line in lines:
        if line.startswith("diff --git "):
            parts = line.split()
            if len(parts) >= 4:
                return _strip_diff_prefix(parts[3])
        if line.startswith("+++ "):
            path = line[4:].strip()
            if path != "/dev/null":
                return _strip_diff_prefix(path)
    return ""


def _split_diff_sections(diff_text: str) -> list[list[str]]:
    sections: list[list[str]] = []
    current: list[str] = []
    for line in (diff_text or "").splitlines():
        if line.startswith("diff --git "):
            if current:
                sections.append(current)
            current = [line]
        elif current:
            current.append(line)
    if current:
        sections.append(current)
    return sections


def _review_diff_text(files: list[dict[str, Any]], original_files: list[dict[str, Any]], diff_text: str) -> str:
    if len(files) == len(original_files or []):
        return diff_text
    patch_text = "\n".join(str(file_info.get("patch") or "") for file_info in files if file_info.get("patch"))
    sections = _split_diff_sections(diff_text)
    if not sections:
        return patch_text
    allowed_paths = {_changed_file_path(file_info) for file_info in files}
    filtered = ["\n".join(section) for section in sections if _diff_section_path(section) in allowed_paths]
    return "\n".join(filtered) if filtered else patch_text


# Defaults mirror config["context"]; kept here so build_pr_context stays usable
# without a fully-populated config (e.g. local dry-runs and unit tests).
_DEFAULT_DIFF_SUMMARY_TOKENS = 4000
_DEFAULT_PER_FILE_PATCH_TOKENS = 3000
_DEFAULT_TOTAL_PATCH_TOKENS = 24000


def _budget_changed_file_patches(files: list[dict[str, Any]], per_file_tokens: int, total_tokens: int) -> tuple[list[dict[str, Any]], dict[str, Any]]:
    """Bound the per-file ``patch`` bodies embedded in the PR context.

    A large PR otherwise inlines every file's full patch (unbounded) into every
    downstream model prompt. Oversized patches are reduced to their hunk headers and
    flagged with ``patch_truncated`` so the model knows the body was dropped. The
    changed-line map is built from the *full* patches by the caller, so inline-finding
    enforcement is unaffected by this truncation.
    """
    out: list[dict[str, Any]] = []
    remaining = max(0, int(total_tokens))
    truncated_files = 0
    for f in files or []:
        nf = dict(f)
        patch = nf.get("patch")
        if isinstance(patch, str) and patch:
            patch_tokens = estimate_tokens(patch)
            if patch_tokens > per_file_tokens or patch_tokens > remaining:
                headers = hunk_headers(patch)
                nf["patch"] = headers
                nf["patch_truncated"] = True
                nf["original_patch_tokens"] = patch_tokens
                remaining -= min(estimate_tokens(headers), remaining)
                truncated_files += 1
            else:
                remaining -= patch_tokens
        out.append(nf)
    return out, {"patches_truncated": truncated_files > 0, "truncated_patch_count": truncated_files}


def build_pr_context(event: dict[str, Any], pr: dict[str, Any], files: list[dict[str, Any]], diff: str, config: dict[str, Any]) -> dict[str, Any]:
    head=pr.get("head", {}) or {}; base=pr.get("base", {}) or {}
    ctx_budget = (config or {}).get("context", {}) or {}
    diff_summary_chars = tokens_to_chars(int(ctx_budget.get("diff_summary_tokens", _DEFAULT_DIFF_SUMMARY_TOKENS)))
    code_files = _review_code_files(files)
    diff_text = _review_diff_text(code_files, files, diff or "")
    budgeted_files, patch_stats = _budget_changed_file_patches(
        code_files,
        int(ctx_budget.get("per_file_patch_tokens", _DEFAULT_PER_FILE_PATCH_TOKENS)),
        int(ctx_budget.get("total_patch_tokens", _DEFAULT_TOTAL_PATCH_TOKENS)),
    )
    context={
        "schema_version": "pr-context.v1",
        "repository": event.get("repository", {}).get("full_name") or pr.get("base", {}).get("repo", {}).get("full_name"),
        "owner": (event.get("repository", {}).get("owner", {}) or {}).get("login"),
        "pr_number": pr.get("number") or event.get("number") or event.get("pull_request", {}).get("number"),
        "title": pr.get("title") or event.get("pull_request", {}).get("title"),
        "body": pr.get("body") or "",
        "state": pr.get("state"),
        "base_ref": base.get("ref"),
        "base_sha": base.get("sha"),
        "head_ref": head.get("ref"),
        "head_sha": head.get("sha"),
        "head_repo_full_name": (head.get("repo") or {}).get("full_name"),
        "base_repo_full_name": (base.get("repo") or {}).get("full_name"),
        "same_repo": (head.get("repo") or {}).get("full_name") == (base.get("repo") or {}).get("full_name") if head.get("repo") and base.get("repo") else None,
        # changed_line_map is derived from the FULL diff before patch budgeting so
        # inline-comment validation keeps the complete set of changed right-side lines.
        "changed_files": budgeted_files,
        "changed_line_map": serialize_changed_line_map(build_changed_line_map(code_files)),
        "diff_summary": summarize_diff(diff_text, diff_summary_chars),
        "diff_truncated": len(diff_text) > diff_summary_chars,
        "patches_truncated": patch_stats["patches_truncated"],
        "truncated_patch_count": patch_stats["truncated_patch_count"],
        "config_base_branch": config.get("base_branch"),
    }
    return include_changed_files_summary(include_repository_metadata(context))


def include_repository_metadata(context: dict[str, Any]) -> dict[str, Any]:
    # build_pr_context always seeds "owner" (often None when there is no GitHub
    # event payload, e.g. the repository_dispatch loop), so setdefault would not
    # backfill it. Fill owner/repo from the repository slug whenever they are
    # falsy, not just missing.
    repo=context.get("repository") or context.get("base_repo_full_name")
    if repo and "/" in repo:
        owner_part, repo_part = repo.split("/",1)
        if not context.get("owner"):
            context["owner"] = owner_part
        if not context.get("repo"):
            context["repo"] = repo_part
    return context


def include_changed_files_summary(context: dict[str, Any]) -> dict[str, Any]:
    summary=[]
    for f in context.get("changed_files", []) or []:
        if is_memory_path(_changed_file_path(f)):
            continue
        summary.append({"filename": f.get("filename") or f.get("path"), "status": f.get("status"), "additions": f.get("additions", 0), "deletions": f.get("deletions", 0)})
    context["changed_files_summary"] = summary
    return context


def context_truncation_evidence(context: dict[str, Any]) -> dict[str, Any] | None:
    """Return a compact truncation summary when PR context dropped diff/patch content.

    Callers use this to emit an audit signal so incomplete coverage is visible rather
    than silent. Returns ``None`` when nothing was truncated.
    """
    if not (context.get("diff_truncated") or context.get("patches_truncated")):
        return None
    return {
        "diff_truncated": bool(context.get("diff_truncated")),
        "patches_truncated": bool(context.get("patches_truncated")),
        "truncated_patch_count": int(context.get("truncated_patch_count") or 0),
        "pr_number": context.get("pr_number"),
        "head_sha": context.get("head_sha"),
    }


def write_pr_context(context: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, context, "pr-context.v1")
