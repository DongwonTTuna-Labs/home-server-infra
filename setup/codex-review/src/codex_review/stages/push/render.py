"""Stage07 render helpers."""
from __future__ import annotations
from typing import Any

def render_test_summary(test_report: dict[str, Any]) -> str:
    lines=["## Test report", f"Passed: {test_report.get('passed')}"]
    for r in test_report.get("results", []): lines.append(f"- `{r.get('command')}`: {r.get('returncode')}")
    return "\n".join(lines)+"\n"

def render_push_summary(push_result: dict[str, Any]) -> str:
    return f"## Push result\n\nPushed: {push_result.get('pushed')}\nCommit: `{push_result.get('commit_sha','')}`\n"

def render_blocked_push_reason(validation_report: dict[str, Any]) -> str:
    return f"Push blocked: {validation_report.get('reason') or validation_report}"
