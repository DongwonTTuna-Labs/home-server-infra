"""CLI handler: schema commands."""
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


def handle_schema(args: argparse.Namespace) -> tuple[Any, str | None]:
    if args.command != "openai-strict":
        raise ValueError(f"unknown schema command: {args.command}")
    if not args.schema:
        raise ValidationError("schema openai-strict requires --schema")
    from codex_review.core.schema import load_schema_json, make_openai_structured_output_schema
    return make_openai_structured_output_schema(load_schema_json(args.schema)), None
