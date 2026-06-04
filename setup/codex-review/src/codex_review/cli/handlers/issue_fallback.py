"""CLI handler: issue_fallback commands."""
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


def handle_issue_fallback(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "plan":
        from codex_review.stages.issue_fallback.issue import build_issue_fallback_plan
        payload = _json_or_default(args.in_path, {})
        reason = args.mode or payload.get("reason") or payload.get("route") or payload.get("status") or "manual_fallback"
        attempted = payload.get("attempted_stages") if isinstance(payload.get("attempted_stages"), list) else []
        deferred_items = payload.get("deferred_items") if isinstance(payload.get("deferred_items"), list) else []
        if deferred_items and not attempted:
            attempted = ["techlead_defer_to_issue"]
        return build_issue_fallback_plan(
            reason=str(reason),
            pr_context=_maybe_json(args.pr_context, {}),
            openspec_context=_json_or_default(args.openspec_context, {}),
            attempted_stages=attempted,
            deferred_items=deferred_items,
        ), "issue-fallback-issue-fallback.v1"
    if cmd == "infer-reason":
        from codex_review.stages.issue_fallback.issue import infer_issue_reason
        return infer_issue_reason(
            review_publication=_json_or_default(args.review_context, {}),
            design_route=_json_or_default(args.chief_decision, {}),
            fix_validation=_json_or_default(args.validation, {}),
            fallback_reason=args.mode,
        ), None
    if cmd == "build-prompt":
        from codex_review.stages.issue_fallback.issue import build_issue_content_prompt
        return build_issue_content_prompt(_maybe_json(args.in_path, {})), None
    if cmd == "compose":
        from codex_review.stages.issue_fallback.issue import build_issue_content_prompt, compose_issue_content
        plan = _maybe_json(args.in_path, {})
        if args.result:
            content = _json_or_default(args.result, {})
        else:
            from codex_review.stages.issue_fallback.issue import CONTENT_SCHEMA_VERSION
            prompt_path = args.prompt
            if not prompt_path and args.prompt_out:
                from codex_review.model.adapter import write_prompt_if_needed
                prompt_path = str(write_prompt_if_needed(build_issue_content_prompt(plan), args.prompt_out))
            content = _model_or_fallback(
                argparse.Namespace(**{**vars(args), "prompt": prompt_path}),
                stage="issue_fallback",
                expected_schema=CONTENT_SCHEMA_VERSION,
                fallback={"title": plan.get("title"), "body": plan.get("body")},
            )
        return compose_issue_content(plan, content), "issue-fallback-issue-fallback.v1"
    if cmd == "apply":
        from codex_review.stages.issue_fallback.issue import apply_issue_fallback
        return apply_issue_fallback(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), args.token, dry_run=args.dry_run), "issue-fallback-issue-fallback.v1"
    raise ValueError(f"unknown issue_fallback command: {cmd}")
