"""CLI handler: design_chief commands."""
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


def handle_design_chief(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"default-result", "model-result"}:
        plan = _maybe_json(args.design_plan or args.in_path, {})
        status = "needs_human" if plan.get("requires_human_review") else ("no_fix_needed" if not plan.get("edit_sequence") else "needs_human")
        fallback = {
            "schema_version": "design-chief-decision.v1",
            "status": status,
            "reason": "safe deterministic default",
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design chief model result was available."),
            "defaulted": True,
        }
        if cmd == "model-result":
            return _model_or_fallback(args, stage="design_chief", expected_schema="design-chief-decision.v1", fallback=fallback), "design-chief-decision.v1"
        return fallback, "design-chief-decision.v1"
    if cmd == "build-chief-prompt":
        from codex_review.stages.design_chief.prompt import build_design_chief_prompt
        memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
        return build_design_chief_prompt(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {}), _maybe_json(args.pr_context, {}), config, memory_context=memory_context), None
    if cmd == "validate":
        from codex_review.stages.design_chief.validate import validate_chief_decision
        return validate_chief_decision(_maybe_json(args.in_path, {}), _maybe_json(args.inventory or args.design_plan, {}), config, args.repo_path), "design-chief-decision.v1"
    if cmd == "route":
        from codex_review.stages.design_chief.route import route_after_design_chief, write_chief_route_outputs
        route = route_after_design_chief(_maybe_json(args.in_path, {}))
        write_chief_route_outputs(route)
        return route, None
    if cmd == "publish":
        from codex_review.stages.design_chief.publish import publish_design_summary
        token = None if args.dry_run else args.token
        return publish_design_summary(_maybe_json(args.inventory or args.design_plan, {}), _maybe_json(args.in_path, {}), token, _maybe_json(args.pr_context, {}), dry_run=args.dry_run), None
    if cmd == "render":
        from codex_review.stages.design_chief.render import render_chief_decision_markdown
        return render_chief_decision_markdown(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown design_chief command: {cmd}")
