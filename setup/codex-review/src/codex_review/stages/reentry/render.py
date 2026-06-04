"""Stage08 rendering."""
from __future__ import annotations
from typing import Any

def render_reentry_summary(record: dict[str, Any]) -> str:
    return f"## Reentry\n\nPushed: {record.get('pushed')}\nNext entry: `{record.get('next_entry')}`\n"

def render_no_push_reentry_summary(reason: str) -> str:
    return f"No reentry scheduled because push did not occur: {reason}\n"
