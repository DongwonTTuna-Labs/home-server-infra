"""CLI handler: techlead commands."""
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


def handle_techlead(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd in {"default-result", "model-result"}:
        combined = _maybe_json(args.inventory or args.in_path, {"findings": []})
        decisions = [{"finding_id": f.get("finding_id") or f.get("id"), "action": "publish_only", "reason": "safe deterministic default"} for f in combined.get("findings", [])]
        fallback = {
            "schema_version": "techlead-decision.v1",
            "decisions": decisions,
            "needs_design": False,
            "status": "ready" if decisions else "lgtm",
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No techlead model result was available."),
            "defaulted": True,
        }
        if cmd == "model-result":
            return _model_or_fallback(args, stage="techlead", expected_schema="techlead-decision.v1", fallback=fallback), "techlead-decision.v1"
        return fallback, "techlead-decision.v1"
    if cmd == "filter-resolved":
        from codex_review.stages.techlead.filter_resolved import filter_findings_against_resolved
        combined = _maybe_json(args.in_path, {"findings": []})
        resolved_memory = _json_or_default(args.inventory, {})
        pr = _json_or_default(args.pr_context, {})
        changed_line_map = pr.get("changed_line_map") or {}
        filtered, _ = filter_findings_against_resolved(combined, resolved_memory, changed_line_map, config)
        return filtered, "review-combined-findings.v1"
    if cmd == "build-techlead-prompt":
        from codex_review.stages.techlead.prompt import build_techlead_prompt
        memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
        return build_techlead_prompt(
            _maybe_json(args.in_path, {}),
            _maybe_json(args.pr_context, {}),
            _maybe_text(args.review_context),
            _maybe_text(args.docs_context),
            config,
            memory_context=memory_context,
        ), None
    if cmd == "validate":
        from codex_review.stages.techlead.validate import validate_techlead_decision
        combined = _maybe_json(args.artifacts[0], {}) if args.artifacts else _maybe_json(args.inventory, {})
        return validate_techlead_decision(_maybe_json(args.in_path, {}), combined, config, args.repo_path), "techlead-decision.v1"
    if cmd == "classify":
        from codex_review.stages.techlead.classify import build_review_publication
        combined = _maybe_json(args.artifacts[0], {}) if args.artifacts else _maybe_json(args.inventory, {})
        return build_review_publication(_maybe_json(args.in_path, {}), combined, config), "techlead-review-publication.v1"
    if cmd == "write-deferred-outputs":
        payload = _maybe_json(args.in_path, {})
        count = len(payload.get("deferred_items") or [])
        write_output("has_deferred_issue_items", "true" if count else "false")
        write_output("deferred_issue_count", str(count))
        return {"has_deferred_issue_items": bool(count), "deferred_issue_count": count}, None
    if cmd == "publish":
        from codex_review.stages.techlead.publish import publish_review
        changed_payload = _json_or_default(args.changed_lines, {})
        changed = changed_payload.get("changed_line_map", changed_payload) if isinstance(changed_payload, dict) else {}
        token = None if args.dry_run else args.token
        return publish_review(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), changed, token, config, dry_run=args.dry_run), None
    if cmd == "render":
        from codex_review.stages.techlead.render import render_review_body
        return render_review_body(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown techlead command: {cmd}")
