"""CLI handler: reentry commands."""
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


def handle_reentry(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "record-reentry":
        from codex_review.stages.reentry.record import build_reentry_record, persist_reentry_loop_state
        push_result = _maybe_json(args.in_path, {})
        loop_state = _json_or_default(args.loop_state, {}) or _maybe_json(args.inventory, {})
        record = build_reentry_record(push_result, loop_state, {"push_result": push_result})
        record = persist_reentry_loop_state(record, _maybe_json(args.pr_context, {}), args.token)
        return record, "reentry-loop-state.v1"
    if cmd == "validate":
        from codex_review.stages.reentry.validate import validate_reentry_record
        return validate_reentry_record(_maybe_json(args.in_path, {}), _json_or_default(args.loop_state, {})), "reentry-loop-state.v1"
    if cmd == "route":
        from codex_review.stages.reentry.route import determine_reentry_expectation
        return determine_reentry_expectation(_maybe_json(args.in_path, {})), None
    if cmd == "render":
        from codex_review.stages.reentry.render import render_reentry_summary
        return render_reentry_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown reentry command: {cmd}")
