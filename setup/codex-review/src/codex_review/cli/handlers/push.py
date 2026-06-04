"""CLI handler: push commands."""
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


def handle_push(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "validate-current-head":
        from codex_review.stages.push.validate import validate_current_head
        validate_current_head(_maybe_json(args.pr_context, {}), _maybe_json(args.in_path, {}), args.token)
        return {"ok": True}, None
    if cmd == "apply-patch":
        from codex_review.stages.push.apply_patch import apply_merged_patch
        return apply_merged_patch(args.patch or args.in_path, args.repo_path), None
    if cmd == "run-tests":
        from codex_review.stages.push.run_tests import run_required_tests, select_test_commands
        return run_required_tests(select_test_commands(_maybe_json(args.in_path, {}), config), args.repo_path), None
    if cmd == "validate-fix":
        from codex_review.stages.push.orchestrate import validate_and_test_fix
        return validate_and_test_fix(
            _maybe_json(args.in_path, {}),
            _maybe_json(args.pr_context, {}),
            config,
            args.repo_path,
            dry_run=args.dry_run,
            semantic_safety=_json_or_default(args.semantic_safety, {}),
        ), "push-validated-fix.v1"
    if cmd == "commit":
        from codex_review.stages.push.orchestrate import commit_validated_fix
        return commit_validated_fix(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), config, args.repo_path), None
    if cmd == "commit-push":
        from codex_review.stages.push.orchestrate import commit_and_push_validated_fix
        return commit_and_push_validated_fix(_maybe_json(args.in_path, {}), _maybe_json(args.validation, {}), _maybe_json(args.pr_context, {}), config, args.repo_path, args.token, dry_run=args.dry_run), "push-result.v1"
    if cmd == "push":
        from codex_review.stages.push.orchestrate import run_push_flow
        return run_push_flow(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), config, args.repo_path, args.token, dry_run=args.dry_run), "push-result.v1"
    if cmd == "check-loop-budget":
        from codex_review.loop.state import build_push_entry, detect_oscillation, read_loop_state_artifact
        validated = _maybe_json(args.in_path, {})
        # Only a patch that WOULD push can continue an oscillation; otherwise pass through.
        if not validated.get("validated"):
            return {**validated, "loop_budget_ok": True}, "push-validated-fix.v1"
        merged = _json_or_default(args.result, {})
        patch = merged.get("patch") or merged.get("patch_text") or ""
        if not patch and merged.get("patch_path"):
            patch = _maybe_text(str(merged["patch_path"]))
        design_plan = _json_or_default(args.design_plan, {})
        prior = read_loop_state_artifact(args.loop_state)
        candidate = build_push_entry(int(prior.get("round_count", 0)) + 1, validated, patch, design_plan)
        verdict = detect_oscillation(prior, candidate, config)
        if verdict["ok"]:
            return {**validated, "loop_budget_ok": True}, "push-validated-fix.v1"
        return {
            **validated,
            "status": verdict["status"],
            "validated": False,
            "pushed": False,
            "loop_budget_ok": False,
            "loop_budget_status": verdict["status"],
            "loop_budget_reason": verdict["reason"],
        }, "push-validated-fix.v1"
    if cmd == "record-push":
        from codex_review.loop.state import append_push_to_loop_state, build_push_entry, read_loop_state_artifact
        push_result = _maybe_json(args.in_path, {})
        prior = read_loop_state_artifact(args.loop_state)
        # Only successful pushes extend the history; anything else is a no-op pass-through.
        if not push_result.get("pushed"):
            return prior, "loop-state.v1"
        merged = _json_or_default(args.result, {})
        patch = merged.get("patch") or merged.get("patch_text") or ""
        if not patch and merged.get("patch_path"):
            patch = _maybe_text(str(merged["patch_path"]))
        design_plan = _json_or_default(args.design_plan, {})
        prior = read_loop_state_artifact(args.loop_state)
        window = int(config.get("autofix", {}).get("oscillation_window", 10))
        entry = build_push_entry(int(prior.get("round_count", 0)) + 1, push_result, patch, design_plan)
        next_state = append_push_to_loop_state(prior, entry, window)
        return next_state, "loop-state.v1"
    if cmd == "write-validation-outputs":
        payload = _maybe_json(args.in_path, {})
        status = str(payload.get("status") or "unknown")
        validated = bool(payload.get("validated"))
        requires_push_token = validated
        # A validated patch will push, so the loop should re-review afterwards.
        # Terminal loop reasons (oscillation / round cap) stop the loop and route to the issue workflow.
        terminal_reasons = {"oscillation_detected", "max_rounds_reached", "no_fix_changes", "no_diff_repeat", "no-diff-repeat"}
        loop_terminal_reason = "no_fix_changes" if (not validated and status in {"no_diff_repeat", "no-diff-repeat"}) else status if (not validated and status in terminal_reasons) else ""
        should_continue = validated
        write_output("validation_status", status)
        write_output("requires_push_token", str(requires_push_token).lower())
        write_output("loop_terminal_reason", loop_terminal_reason)
        write_output("should_continue", str(should_continue).lower())
        return {
            "validation_status": status,
            "requires_push_token": requires_push_token,
            "loop_terminal_reason": loop_terminal_reason,
            "should_continue": should_continue,
        }, None
    if cmd == "write-outputs":
        payload = _maybe_json(args.in_path, {})
        status = str(payload.get("status") or "unknown")
        write_output("push_status", status)
        return {"push_status": status}, None
    if cmd == "render":
        from codex_review.stages.push.render import render_push_summary
        return render_push_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown push command: {cmd}")
