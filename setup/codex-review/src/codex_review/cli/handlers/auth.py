"""CLI handler: auth commands."""
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


def handle_auth(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.github.app_token import create_installation_token_for_repo, permissions_for_write_mode
    if args.command != "app-token":
        raise ValueError(f"unknown auth command: {args.command}")
    pr = _maybe_json(args.pr_context, {})
    owner, repo = _repo_parts_from_context(pr)
    if not owner or not repo:
        raise ValidationError("auth app-token requires owner/repo from --pr-context")
    mode = args.mode or args.stage or "write"
    permissions = permissions_for_write_mode(mode)
    token = create_installation_token_for_repo(owner, repo, permissions)
    mask_secret(token)
    permissions_json = json.dumps(permissions, sort_keys=True, separators=(",", ":"))
    if os.environ.get("GITHUB_OUTPUT"):
        write_output("token", token)
        write_output("token_created", "true")
        write_output("permissions_json", permissions_json)
    return {"schema_version":"github-app-token.v1", "token_created": True, "owner": owner, "repo": repo, "permissions": permissions, "permissions_json": permissions_json, "repository_scoped": True}, None
