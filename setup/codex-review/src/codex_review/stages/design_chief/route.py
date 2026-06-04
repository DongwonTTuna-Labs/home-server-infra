"""Route after design chief."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.core.output import write_output

def route_after_design_chief(chief_decision: dict[str, Any]) -> dict[str, Any]:
    status=chief_decision.get("status")
    route="run_fix_dispatch" if status=="approved_for_fix" else ("stop_needs_human" if status=="needs_human" else "stop_noop")
    return {"schema_version":"design-chief-route.v1","route":route,"status":status}

def write_chief_route_outputs(route: dict[str, Any]) -> None:
    write_output("route", route.get("route")); write_output("run_fix_dispatch", route.get("route")=="run_fix_dispatch")

def write_design_chief_route_artifact(route: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, route, "design-chief-route.v1")
