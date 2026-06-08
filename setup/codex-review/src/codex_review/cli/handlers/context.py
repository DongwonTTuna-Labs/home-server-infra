"""CLI handler: context commands."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, read_text, write_json, write_text
from codex_review.core.config import load_config
from codex_review.core.env import read_event_payload
from codex_review.core.errors import CodexReviewError, ValidationError, format_error
from codex_review.core.output import append_step_summary, mask_secret, write_output
from codex_review.cli._helpers import (
    _add_common, _artifact_paths, _default_inspection_evidence, _emit,
    _json_or_default, _maybe_json, _maybe_text, _model_or_fallback,
    _preferred_artifact_paths, _repo_parts_from_context, _safe_path_component,
)
from codex_review.memory.paths import is_memory_path


def _filter_changed_line_map(changed: dict[str, Any]) -> dict[str, Any]:
    return {str(path): lines for path, lines in changed.items() if not is_memory_path(str(path))}


def _signal_context_truncation(context: dict[str, Any]) -> None:
    """Surface PR-context truncation so dropped coverage is visible, not silent."""
    from codex_review.context.pr import context_truncation_evidence

    evidence = context_truncation_evidence(context)
    if not evidence:
        return
    write_output("context_truncated", "true")
    write_output("context_truncated_patch_count", str(evidence["truncated_patch_count"]))
    append_step_summary(
        "> [!WARNING] PR context exceeded token budget and was truncated: "
        f"diff_truncated={evidence['diff_truncated']}, "
        f"patches_truncated={evidence['patches_truncated']} "
        f"({evidence['truncated_patch_count']} file patches reduced to hunk headers). "
        "Review coverage of the dropped content may be incomplete."
    )
    artifact_root = os.environ.get("CODEX_REVIEW_ARTIFACT_ROOT")
    if artifact_root:
        try:
            from codex_review.loop.events import append_event_log, record_event

            append_event_log(Path(artifact_root) / "events.jsonl", record_event("CONTEXT_TRUNCATED", "context.pr", evidence))
        except Exception:
            pass


def handle_context(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "pr":
        from codex_review.context.pr import build_pr_context
        from codex_review.github.pull_requests import get_pull_request, list_pull_request_files
        event_payload = read_event_payload(args.event) if args.event else _json_or_default(args.in_path, {})
        event_ctx = _json_or_default(args.pr_context, {})
        owner, repo = _repo_parts_from_context(event_ctx or event_payload)
        pr_number = event_ctx.get("pr_number") or event_payload.get("number") or (event_payload.get("pull_request") or {}).get("number") or (event_payload.get("inputs") or {}).get("pr_number") or os.environ.get("CODEX_REVIEW_PR_NUMBER")
        pr = event_payload.get("pull_request") or {}
        files: list[dict[str, Any]] = []
        if owner and repo and pr_number and args.token:
            try:
                pr = get_pull_request(owner, repo, int(pr_number), args.token)
                files = list_pull_request_files(owner, repo, int(pr_number), args.token)
            except CodexReviewError:
                raise
            except Exception:
                # Local dry-runs and tests can still construct context from the event payload.
                files = []
        if not pr:
            pr = event_payload.get("pull_request") or event_ctx
        diff = "\n".join(str(f.get("patch") or "") for f in files)
        context = build_pr_context(event_payload if isinstance(event_payload, dict) else {}, pr, files, diff, config)
        _signal_context_truncation(context)
        return context, "pr-context.v1"
    if cmd == "changed-lines":
        from codex_review.context.diff import build_changed_line_map, serialize_changed_line_map
        payload = _json_or_default(args.in_path, {})
        if not payload and args.pr_context:
            payload = _json_or_default(args.pr_context, {})
        changed = payload.get("changed_line_map") if isinstance(payload, dict) and payload.get("changed_line_map") else build_changed_line_map(payload)
        if isinstance(changed, dict):
            changed = _filter_changed_line_map(changed)
        return {"schema_version": "changed-lines.v1", "changed_line_map": serialize_changed_line_map(changed)}, None
    if cmd == "docs":
        from codex_review.context.docs import find_repository_docs, read_docs_with_budget, render_docs_context
        docs = read_docs_with_budget(find_repository_docs(args.repo_path), int(config.get("docs_context_budget", 20000)))
        return render_docs_context(docs), None
    if cmd == "openspec":
        from codex_review.context.openspec import collect_openspec_context
        pr = _json_or_default(args.pr_context or args.in_path, {})
        return collect_openspec_context(pr, args.repo_path, args.token), "openspec-context.v1"
    if cmd == "openspec-markdown":
        from codex_review.context.openspec import render_openspec_context_markdown, sections_for_stage
        budget = int((config.get("context", {}) or {}).get("openspec_tokens", 0)) or None
        return render_openspec_context_markdown(
            _maybe_json(args.in_path or args.openspec_context, {}),
            sections=sections_for_stage(args.stage),
            budget_tokens=budget,
        ), None
    if cmd == "openspec-outputs":
        payload = _maybe_json(args.in_path or args.openspec_context, {})
        outputs = {
            "openspec_present": str(bool(payload.get("present"))).lower(),
            "openspec_status": str(payload.get("status") or ""),
            "openspec_decision": str(payload.get("decision") or ""),
        }
        for key, value in outputs.items():
            write_output(key, value)
        return {"outputs": outputs}, None
    if cmd == "memory":
        from codex_review.memory.context import build_memory_context_markdown
        pr = _json_or_default(args.pr_context or args.in_path, {})
        return build_memory_context_markdown(pr, args.repo_path, config), None
    if cmd == "review":
        from codex_review.context.threads import build_review_context_markdown
        pr = _json_or_default(args.pr_context, {})
        threads_payload = _json_or_default(args.in_path, [])
        threads = threads_payload.get("threads") if isinstance(threads_payload, dict) else threads_payload
        return build_review_context_markdown(pr, threads or [], [], []), None
    raise ValueError(f"unknown context command: {cmd}")
