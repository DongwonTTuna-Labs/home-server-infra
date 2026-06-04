"""CLI handler: resolve_gate commands."""
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


def handle_resolve_gate(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "collect":
        from codex_review.github.review_threads import collect_review_threads
        from codex_review.stages.resolve_gate.collect import collect_thread_inventory
        pr = _maybe_json(args.pr_context, {})
        raw = _json_or_default(args.in_path, None)
        if raw is None:
            owner, repo = _repo_parts_from_context(pr)
            pr_number = pr.get("pr_number")
            threads = collect_review_threads(owner, repo, int(pr_number), args.token) if owner and repo and pr_number and args.token else []
        else:
            threads = raw.get("threads") or raw.get("review_threads") if isinstance(raw, dict) else raw
        inv = collect_thread_inventory(pr, threads or [], [], config)
        return inv, "resolve-gate-thread-inventory.v1"
    if cmd == "collect-resolved":
        from codex_review.github.review_threads import collect_review_threads
        from codex_review.stages.resolve_gate.collect import build_resolved_memory
        pr = _maybe_json(args.pr_context, {})
        raw = _json_or_default(args.in_path, None)
        if raw is None:
            owner, repo = _repo_parts_from_context(pr)
            pr_number = pr.get("pr_number")
            threads = collect_review_threads(owner, repo, int(pr_number), args.token) if owner and repo and pr_number and args.token else []
        else:
            threads = raw.get("threads") or raw.get("review_threads") if isinstance(raw, dict) else raw
        return build_resolved_memory(pr, threads or [], config), "resolve-gate-resolved-memory.v1"
    if cmd in {"default-result", "model-result"}:
        inv = _maybe_json(args.inventory or args.in_path, {})
        decisions = []
        for item in inv.get("items", []):
            state = "needs_human" if item.get("forced_needs_human") else "current_head_keep_open"
            decisions.append({"thread_id": item.get("thread_id"), "state": state, "reason": item.get("forced_reason") or "safe default keeps existing thread open"})
        fallback = {"schema_version": "resolve-gate-lifecycle-result.v1", "decisions": decisions, "defaulted": True}
        if cmd == "model-result":
            return _model_or_fallback(args, stage="resolve_gate", expected_schema="resolve-gate-lifecycle-result.v1", fallback=fallback), "resolve-gate-lifecycle-result.v1"
        return fallback, "resolve-gate-lifecycle-result.v1"
    if cmd == "build-prompt":
        from codex_review.stages.resolve_gate.prompt import build_lifecycle_prompt
        inv = _maybe_json(args.inventory or args.in_path, {})
        prompt = build_lifecycle_prompt(inv, _maybe_text(args.review_context), _maybe_text(args.docs_context), config)
        return prompt, None
    if cmd == "validate":
        from codex_review.stages.resolve_gate.validate import validate_lifecycle_result
        return validate_lifecycle_result(_maybe_json(args.result or args.in_path, {}), _maybe_json(args.inventory, {})), "resolve-gate-lifecycle-result.v1"
    if cmd == "apply":
        from codex_review.stages.resolve_gate.apply import apply_lifecycle_result
        return apply_lifecycle_result(_maybe_json(args.result or args.in_path, {}), _maybe_json(args.pr_context, {}), args.token, config, dry_run=args.dry_run), None
    if cmd == "route":
        from codex_review.stages.resolve_gate.route import build_gate_result, emit_gate_outputs
        apply_report = _maybe_json(args.artifacts[0], {}) if args.artifacts else {}
        gate = build_gate_result(_maybe_json(args.inventory, {}), _maybe_json(args.result or args.in_path, {}), apply_report)
        emit_gate_outputs(gate)
        return gate, "resolve-gate-result.v1"
    if cmd == "render":
        from codex_review.stages.resolve_gate.render import render_resolve_gate_step_summary
        return render_resolve_gate_step_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown resolve_gate command: {cmd}")
