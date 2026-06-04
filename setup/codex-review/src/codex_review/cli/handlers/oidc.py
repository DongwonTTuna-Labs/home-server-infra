"""CLI handler: oidc commands."""
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


def handle_oidc(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.github.oidc_token import (
        DEFAULT_AUDIENCE,
        DEFAULT_BROKER_URL,
        DEFAULT_RESPONSES_ENDPOINT,
        mint_relay_token,
    )
    if args.command != "relay-token":
        raise ValueError(f"unknown oidc command: {args.command}")
    audience = args.audience or DEFAULT_AUDIENCE
    broker_url = args.broker_url or DEFAULT_BROKER_URL
    credential = mint_relay_token(audience, broker_url)
    mask_secret(credential["relay_token"])
    if os.environ.get("GITHUB_OUTPUT"):
        write_output("relay_token", credential["relay_token"])
        write_output("expires_at", credential["expires_at"])
        write_output("endpoint", DEFAULT_RESPONSES_ENDPOINT)
    return {
        "schema_version": "codex-oidc-relay.v1",
        "relay_token_minted": True,
        "expires_at": credential["expires_at"],
        "endpoint": DEFAULT_RESPONSES_ENDPOINT,
    }, None
