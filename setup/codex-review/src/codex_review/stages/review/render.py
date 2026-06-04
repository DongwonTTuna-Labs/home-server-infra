"""Stage01 render helpers."""
from __future__ import annotations
from typing import Any

def render_axis_summary(axis: str, findings: dict[str, Any]) -> str:
    return f"## {axis} review\n\nFindings: {len(findings.get('findings', []))}\n"

def render_combined_summary(combined: dict[str, Any]) -> str:
    lines=["## Combined review findings", f"Total: {combined.get('finding_count', len(combined.get('findings', [])))}"]
    for axis,count in (combined.get("summary_by_axis") or {}).items(): lines.append(f"- {axis}: {count}")
    return "\n".join(lines)+"\n"
