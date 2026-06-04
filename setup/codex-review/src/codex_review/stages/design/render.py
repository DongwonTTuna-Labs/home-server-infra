"""Stage03 rendering."""
from __future__ import annotations
from typing import Any

def render_design_plan_markdown(plan: dict[str, Any]) -> str:
    lines=["## Design plan", f"Hash: `{plan.get('plan_hash','')}`", "", "### Edit sequence"]
    for step in plan.get("edit_sequence", []): lines.append(f"- {step.get('task_id') or step.get('id')}: {step.get('summary') or step}")
    lines.append("\n### Tests")
    for t in plan.get("tests", []): lines.append(f"- {t}")
    return "\n".join(lines)+"\n"

def render_design_step_summary(plan: dict[str, Any], validation: dict[str, Any]) -> str:
    return f"## Stage03 design\n\nPlan hash: `{plan.get('plan_hash')}`\nValidation: {validation}\n"
