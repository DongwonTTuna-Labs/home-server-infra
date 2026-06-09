"""CLI handler: review commands."""
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


def handle_review(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "axes":
        from codex_review.stages.review.axes import review_axes
        return {"axes": review_axes(config)}, None
    if cmd in {"default-result", "model-result"}:
        axis = args.axis or "correctness"
        fallback = {
            "schema_version": "review-axis-findings.v1",
            "axis": axis,
            "findings": [],
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No model result was available for this axis."),
            "defaulted": True,
        }
        if cmd == "model-result":
            return _model_or_fallback(args, stage=f"review_{axis}", expected_schema="review-axis-findings.v1", fallback=fallback), "review-axis-findings.v1"
        return fallback, "review-axis-findings.v1"
    if cmd == "build-review-prompt":
        from codex_review.stages.review.prompt import build_axis_prompt
        memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
        return build_axis_prompt(args.axis or "correctness", _maybe_json(args.pr_context, {}), _maybe_text(args.review_context), _maybe_text(args.docs_context), config, memory_context), None
    if cmd == "validate":
        from codex_review.stages.review.validate import validate_axis_findings
        payload = _maybe_json(args.in_path, {})
        changed_payload = _json_or_default(args.changed_lines, {})
        changed = changed_payload.get("changed_line_map", changed_payload) if isinstance(changed_payload, dict) else {}
        return validate_axis_findings(args.axis or payload.get("axis"), payload, _maybe_json(args.pr_context, {}), changed, config, args.repo_path), "review-axis-findings.v1"
    if cmd == "combine":
        from codex_review.stages.review.combine import combine_axis_findings
        paths = _preferred_artifact_paths(args.artifacts, primary="findings.validated.json", fallback="findings.json")
        return combine_axis_findings(paths, config), "review-combined-findings.v1"
    if cmd == "render":
        from codex_review.stages.review.render import render_combined_summary
        return render_combined_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown review command: {cmd}")
