"""Stage00 gate routing."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.constants import NON_TERMINAL_LIFECYCLE_STATES, TERMINAL_LIFECYCLE_STATES
from codex_review.core.output import set_route_outputs


def determine_route(gate_context: dict[str, Any]) -> str:
    decisions=gate_context.get("decisions") or []
    lifecycle=gate_context.get("lifecycle_result") or {}
    inventory=gate_context.get("inventory") or {}
    if lifecycle.get("route") in {"run_review", "stop_lgtm", "stop_needs_human", "run_design_from_existing_threads", "stop_noop"}:
        return lifecycle["route"]
    if any(d.get("state") == "needs_human" for d in decisions):
        return "stop_needs_human"
    if any(d.get("state") == "fix_now" for d in decisions):
        return "run_design_from_existing_threads"
    unresolved=[d for d in decisions if d.get("state") in NON_TERMINAL_LIFECYCLE_STATES and d.get("state") != "current_head_keep_open"]
    if unresolved:
        return "run_review"
    if lifecycle.get("status") in {"lgtm", "no_unresolved_threads"} and not inventory.get("items"):
        return "stop_lgtm"
    if decisions and all(d.get("state") in TERMINAL_LIFECYCLE_STATES or d.get("state") == "current_head_keep_open" for d in decisions):
        return "run_review"
    if not decisions:
        return "run_review"
    return "stop_noop"


def build_gate_result(inventory: dict[str, Any], lifecycle_result: dict[str, Any], apply_report: dict[str, Any] | None, loop_state: dict[str, Any] | None = None) -> dict[str, Any]:
    decisions=lifecycle_result.get("decisions") or []
    context={"decisions": decisions, "lifecycle_result": lifecycle_result, "inventory": inventory, "apply_report": apply_report or {}, "loop_state": loop_state or {}}
    route=determine_route(context)
    return {"schema_version": "resolve-gate-result.v1", "route": route, "head_sha": inventory.get("head_sha"), "thread_count": len(inventory.get("items", [])), "decisions": decisions, "apply_report": apply_report or {}}


def write_gate_result(gate_result: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, gate_result, "resolve-gate-result.v1")


def emit_gate_outputs(gate_result: dict[str, Any]) -> None:
    set_route_outputs(gate_result)
