"""CLI handler: event commands."""
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


def handle_event(args: argparse.Namespace) -> tuple[Any, str | None]:
    if args.command in {"resolve-current", None}:
        payload = read_event_payload(args.in_path or args.event)
        pr = payload.get("pull_request") or payload
        repo_obj = payload.get("repository") or (pr.get("base") or {}).get("repo") or {}
        repository = repo_obj.get("full_name") or pr.get("base_repo_full_name") or os.environ.get("GITHUB_REPOSITORY")
        owner = ((repo_obj.get("owner") or {}).get("login") if isinstance(repo_obj.get("owner"), dict) else repo_obj.get("owner"))
        repo_name = repo_obj.get("name")
        if repository and "/" in repository:
            owner = owner or repository.split("/", 1)[0]
            repo_name = repo_name or repository.split("/", 1)[1]
        head = pr.get("head") or {}
        base = pr.get("base") or {}
        out = {
            "schema_version": "event-context.v1",
            "event_name": os.environ.get("GITHUB_EVENT_NAME") or payload.get("event_name"),
            "repository": repository,
            "owner": owner,
            "repo": repo_name,
            "pr_number": payload.get("number") or pr.get("number") or (payload.get("inputs") or {}).get("pr_number") or os.environ.get("CODEX_REVIEW_PR_NUMBER"),
            "head_sha": head.get("sha") or pr.get("head_sha"),
            "base_sha": base.get("sha") or pr.get("base_sha"),
            "head_ref": head.get("ref") or pr.get("head_ref"),
            "base_ref": base.get("ref") or pr.get("base_ref"),
            "head_repo_full_name": (head.get("repo") or {}).get("full_name") if isinstance(head.get("repo"), dict) else None,
            "base_repo_full_name": (base.get("repo") or {}).get("full_name") if isinstance(base.get("repo"), dict) else repository,
            "sender": (payload.get("sender") or {}).get("login") if isinstance(payload.get("sender"), dict) else payload.get("sender"),
        }
        if out.get("head_repo_full_name") and out.get("base_repo_full_name"):
            out["same_repo"] = out["head_repo_full_name"] == out["base_repo_full_name"]
        else:
            out["same_repo"] = None
        return out, "event-context.v1"
    if args.command == "write-outputs":
        payload = _maybe_json(args.in_path or args.pr_context, {})
        keys = ["same_repo", "head_sha", "head_repo_full_name", "head_ref", "base_sha", "base_repo_full_name", "pr_number"]
        outputs: dict[str, str] = {}
        for key in keys:
            value = payload.get(key)
            if value is None:
                continue
            text = str(value).lower() if isinstance(value, bool) else str(value)
            outputs[key] = text
            write_output(key, text)
        return {"outputs": outputs}, None
    raise ValueError(f"unknown event command: {args.command}")
