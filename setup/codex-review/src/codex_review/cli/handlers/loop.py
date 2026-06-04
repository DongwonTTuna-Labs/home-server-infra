"""CLI handler: loop commands."""
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


def handle_loop(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.loop.router import route_after_resolve_gate, route_after_techlead, route_after_design_chief, route_after_push, write_route_outputs
    cmd = args.command
    payload = _maybe_json(args.in_path, {})
    if cmd == "route-after-resolve_gate":
        route = route_after_resolve_gate(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-techlead":
        route = route_after_techlead(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-design_chief":
        route = route_after_design_chief(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "route-after-push":
        route = route_after_push(payload)
        write_route_outputs(route)
        return route, None
    if cmd == "summary":
        from codex_review.loop.events import render_event_summary
        return render_event_summary(payload if isinstance(payload, list) else payload.get("events", [])), None
    if cmd == "read-state":
        from codex_review.loop.state import empty_loop_state, read_dispatch_ledger_artifact, read_loop_state_artifact, read_loop_state_payload
        if args.loop_state:
            state = read_loop_state_artifact(args.loop_state)
        elif args.in_path:
            state = read_loop_state_artifact(args.in_path)
        elif isinstance(payload, dict) and payload:
            state = read_loop_state_payload(payload)
        else:
            state = empty_loop_state()
        if args.ledger:
            state["dispatch_ledger"] = read_dispatch_ledger_artifact(args.ledger)["entries"]
        return state, "loop-state.v1"
    if cmd == "append-dispatch-ledger":
        from codex_review.loop.state import append_dispatch_ledger_entry, dispatch_cap_from_config, read_dispatch_ledger_artifact
        config = load_config(args.config)
        ledger = read_dispatch_ledger_artifact(args.ledger)
        window = max(dispatch_cap_from_config(config) * 2, 1)
        return append_dispatch_ledger_entry(ledger, payload, window=window), "dispatch-ledger.v1"
    if cmd == "guard-dispatch":
        from codex_review.loop.state import dispatch_cap_from_config, evaluate_dispatch_ledger, read_dispatch_ledger_artifact
        config = load_config(args.config)
        ledger = read_dispatch_ledger_artifact(args.ledger)
        max_iterations = int(payload.get("max_iterations", 0) or 0)
        if max_iterations <= 0:
            max_iterations = int(config.get("autofix", {}).get("max_rounds", 5))
        return evaluate_dispatch_ledger(
            ledger,
            payload,
            max_iterations=max_iterations,
            dispatch_cap=dispatch_cap_from_config(config),
        ), "dispatch-guard.v1"
    if cmd == "verify-bundle":
        from codex_review.loop.bundle import BUNDLE_VERIFY_SCHEMA, verify_bundle
        directory = args.dir
        if not directory:
            raise ValidationError("loop verify-bundle requires --dir pointing at the downloaded state bundle")
        result = verify_bundle(
            directory,
            args.kind or "auto",
            artifact_name=args.name,
            allow_initial_empty=bool(getattr(args, "allow_initial_empty", False)),
        )
        return result, BUNDLE_VERIFY_SCHEMA
    raise ValueError(f"unknown loop command: {cmd}")
