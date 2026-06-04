"""Stage08 route/output helpers."""
from __future__ import annotations
from typing import Any
from codex_review.core.output import write_output

def determine_reentry_expectation(push_result: dict[str, Any]) -> dict[str, Any]:
    pushed=bool(push_result.get("pushed") or push_result.get("status") == "pushed")
    return {"schema_version":"reentry-route.v1","expect_synchronize":pushed,"next_entry":"resolve_gate" if pushed else "none"}

def write_reentry_outputs(record: dict[str, Any]) -> None:
    write_output("expect_synchronize", record.get("expect_synchronize") or record.get("next_entry") == "resolve_gate_on_synchronize")
    write_output("next_entry", record.get("next_entry"))
