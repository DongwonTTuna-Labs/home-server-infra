"""CLI handler: loop commands."""
from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, read_text, write_json, write_text
from codex_review.core.config import load_config
from codex_review.core.env import read_event_payload
from codex_review.core.errors import CodexReviewError, ValidationError, format_error
from codex_review.core.output import append_step_summary, mask_secret, write_output
from codex_review.memory.paths import is_memory_path
from codex_review.cli._helpers import (
    _add_common, _artifact_paths, _default_inspection_evidence, _emit,
    _json_or_default, _maybe_json, _maybe_text, _model_or_fallback,
    _preferred_artifact_paths, _repo_parts_from_context, _safe_path_component,
)


MEMORY_COMMIT_MARKER = "codex-memory: true"
OWN_ACTORS_ENV = "CODEX_LOOP_OWN_ACTORS"
ACTOR_ENV_NAMES = ("CODEX_LOOP_GITHUB_ACTOR", "GITHUB_ACTOR", "GITHUB_TRIGGERING_ACTOR")
REQUESTER_ENV_NAMES = ("CODEX_LOOP_REQUESTED_BY", "CODEX_REVIEW_REQUESTED_BY")


def classify_memory_only_change(repo_path: str | os.PathLike[str], base: str | None, head: str | None) -> dict[str, Any]:
    base_ref = str(base or "").strip()
    head_ref = str(head or "").strip()
    if not base_ref or not head_ref:
        return _classification_result([], [], "missing_base_or_head", "base and head are required")
    if base_ref.startswith("-") or head_ref.startswith("-"):
        return _classification_result([], [], "invalid_base_or_head", "base and head must be revisions, not options")

    diff_range = f"{base_ref}..{head_ref}"
    try:
        completed = subprocess.run(
            ["git", "-C", str(Path(repo_path)), "diff", "--name-only", diff_range, "--"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except OSError as exc:
        return _classification_result([], [], "git_unavailable", str(exc))

    if completed.returncode != 0:
        detail = completed.stderr.strip() or f"git diff exited {completed.returncode}"
        return _classification_result([], [], "diff_unavailable", detail)

    changed_paths = [line for line in completed.stdout.splitlines() if line]
    non_memory_paths = [path for path in changed_paths if not is_memory_path(path)]
    if not changed_paths:
        return _classification_result(changed_paths, non_memory_paths, "empty_diff", "no changed paths")
    if non_memory_paths:
        return _classification_result(changed_paths, non_memory_paths, "contains_non_memory_changes", "diff includes non-memory paths")

    marker_available, marker_present, marker_detail = _head_commit_marker(repo_path, head_ref)
    if not marker_available:
        return _classification_result(
            changed_paths,
            non_memory_paths,
            "commit_message_unavailable",
            marker_detail,
            codex_memory_marker=False,
        )
    if not marker_present:
        return _classification_result(
            changed_paths,
            non_memory_paths,
            "missing_codex_memory_marker",
            marker_detail,
            codex_memory_marker=False,
        )

    actor_guard, actor_reason, actor_detail = _actor_guard()
    if not actor_guard:
        return _classification_result(
            changed_paths,
            non_memory_paths,
            actor_reason,
            actor_detail,
            codex_memory_marker=True,
            actor_guard=False,
        )

    return _classification_result(
        changed_paths,
        non_memory_paths,
        "memory_only",
        "all changed paths are review-memory paths with codex-memory marker and own actor/requester guard",
        codex_memory_marker=True,
        actor_guard=True,
        actor_guard_detail=actor_detail,
    )


def _head_commit_marker(repo_path: str | os.PathLike[str], head_ref: str) -> tuple[bool, bool, str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(Path(repo_path)), "log", "-1", "--format=%B", head_ref, "--"],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
    except OSError as exc:
        return False, False, str(exc)

    if completed.returncode != 0:
        detail = completed.stderr.strip() or f"git log exited {completed.returncode}"
        return False, False, detail

    message = completed.stdout.strip()
    if MEMORY_COMMIT_MARKER in message.lower():
        return True, True, "head commit has codex-memory marker"
    return True, False, "head commit lacks codex-memory marker"


def _actor_guard() -> tuple[bool, str, str]:
    allowed = _actor_list(os.environ.get(OWN_ACTORS_ENV, ""))
    if not allowed:
        return False, "missing_actor_guard", f"{OWN_ACTORS_ENV} is required for memory-only suppression"

    candidates: list[str] = []
    for name in (*ACTOR_ENV_NAMES, *REQUESTER_ENV_NAMES):
        candidates.extend(_actor_list(os.environ.get(name, "")))
    candidates = list(dict.fromkeys(candidates))
    if not candidates:
        return False, "missing_actor_guard", "github actor or requested_by is required for memory-only suppression"

    if any(not _valid_actor(value) for value in [*allowed, *candidates]):
        return False, "invalid_actor_guard", "actor guard contains malformed actor/requester value"

    matches = sorted(set(allowed) & set(candidates))
    if matches:
        return True, "actor_guard_match", f"matched own actor/requester {matches[0]}"
    return False, "actor_guard_mismatch", "actor/requester is not a configured memory writer"


def _actor_list(value: str) -> list[str]:
    actors: list[str] = []
    for chunk in value.replace("\n", ",").split(","):
        actor = chunk.strip()
        if actor:
            actors.append(actor)
    return actors


def _valid_actor(value: str) -> bool:
    return bool(value) and all(ch.isalnum() or ch in {"-", "_", "[", "]", "@", ".", "/"} for ch in value)


def _classification_result(
    changed_paths: list[str],
    non_memory_paths: list[str],
    reason: str,
    detail: str,
    *,
    codex_memory_marker: bool = False,
    actor_guard: bool = False,
    actor_guard_detail: str = "",
) -> dict[str, Any]:
    memory_only = bool(changed_paths) and not non_memory_paths and reason == "memory_only" and codex_memory_marker and actor_guard
    return {
        "memory_only": memory_only,
        "should_run_model": not memory_only,
        "reason": reason,
        "detail": detail[:500],
        "changed_paths": changed_paths,
        "non_memory_paths": non_memory_paths,
        "changed_path_count": len(changed_paths),
        "non_memory_path_count": len(non_memory_paths),
        "codex_memory_marker": codex_memory_marker,
        "actor_guard": actor_guard,
        "actor_guard_detail": actor_guard_detail[:500],
    }


def _write_memory_classification_outputs(result: dict[str, Any]) -> None:
    write_output("memory_only", "true" if result["memory_only"] else "false")
    write_output("should_run_model", "true" if result["should_run_model"] else "false")
    write_output("classification_reason", result["reason"])
    write_output("changed_path_count", result["changed_path_count"])
    write_output("non_memory_path_count", result["non_memory_path_count"])
    write_output("codex_memory_marker", "true" if result["codex_memory_marker"] else "false")
    write_output("actor_guard", "true" if result["actor_guard"] else "false")


def handle_loop(args: argparse.Namespace) -> tuple[Any, str | None]:
    from codex_review.loop.router import route_after_resolve_gate, route_after_techlead, route_after_design_chief, route_after_push, write_route_outputs
    cmd = args.command
    if cmd == "memory-only-change":
        result = classify_memory_only_change(args.repo_path, args.base, args.head)
        _write_memory_classification_outputs(result)
        return result, None
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
