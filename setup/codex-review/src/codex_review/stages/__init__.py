"""Stage modules for Codex Review v3."""
from __future__ import annotations

STAGE_ORDER = [
    "resolve_gate",
    "review",
    "techlead",
    "design",
    "design_chief",
    "fix_dispatch",
    "fix_merge",
    "push",
    "reentry",
]


def stage_order() -> list[str]:
    return list(STAGE_ORDER)
