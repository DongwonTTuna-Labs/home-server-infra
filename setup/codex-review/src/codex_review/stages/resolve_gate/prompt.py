"""Stage00 lifecycle triage prompt builder."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_text


def render_thread_item_for_prompt(item: dict[str, Any]) -> str:
    return f"""### Thread {item.get('thread_id')}
Path: {item.get('path')}:{item.get('line')}
Root cause key: {item.get('root_cause_key')}
Forced needs human: {item.get('forced_needs_human')} {item.get('forced_reason') or ''}

```text
{item.get('body','')}
```"""


def select_triage_batches(inventory: dict[str, Any], policy: dict[str, Any]) -> list[list[dict[str, Any]]]:
    items=inventory.get("items", [])
    size=int(policy.get("max_threads_per_triage", 16) or 16)
    return [items[i:i+size] for i in range(0, len(items), size)] or [[]]


def build_lifecycle_prompt(inventory: dict[str, Any], review_context: str, docs_context: str, config: dict[str, Any]) -> str:
    items="\n\n".join(render_thread_item_for_prompt(i) for i in inventory.get("items", []))
    return f"""You are the Stage00 Codex review thread lifecycle triager.
Return JSON with schema_version resolve-gate-lifecycle-result.v1 and one decision per thread_id.
Allowed states: {', '.join(config.get('lifecycle', {}).get('terminal_states', []))}, {', '.join(config.get('lifecycle', {}).get('non_terminal_states', []))}
Never resolve current-head or forced_needs_human threads.

{docs_context}

{review_context}

## Thread inventory
{items}
"""


def write_lifecycle_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
