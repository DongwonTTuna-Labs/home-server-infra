"""Stage05 render helpers."""
from __future__ import annotations
from typing import Any

def render_task_manifest_summary(manifest: dict[str, Any]) -> str:
    lines=["## Fix task manifest", f"Tasks: {len(manifest.get('tasks', []))}"]
    for t in manifest.get("tasks", []): lines.append(f"- {t.get('task_id')}: {t.get('summary')}")
    return "\n".join(lines)+"\n"

def render_agent_result_summary(collection: dict[str, Any]) -> str:
    return f"## Fix agent results\n\nReady for merge: {collection.get('ready_for_merge')}\nStatus counts: {collection.get('status_counts')}\n"
