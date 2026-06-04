"""Stage04 render helpers."""
from __future__ import annotations
from typing import Any

def render_chief_decision_markdown(decision: dict[str, Any]) -> str:
    return f"## Design chief decision\n\nStatus: `{decision.get('status')}`\nReason: {decision.get('reason','')}\n"

def render_fix_policy_markdown(policy: dict[str, Any]) -> str:
    lines=["## Fix policy"]
    for k,v in policy.items(): lines.append(f"- {k}: `{v}`")
    return "\n".join(lines)+"\n"
