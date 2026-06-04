"""Fix merge prompt."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_text
from codex_review.context.budget import compact_json

def include_final_patch_contract(prompt: str) -> str:
    return prompt + (
        "\nReturn fix-merge-merged-fix.v1 JSON with a single consolidated `edits` array of "
        "{path, old_str, new_str} search/replace objects (NOT a unified diff). old_str must match "
        "exactly once per file (include surrounding context); an empty old_str creates a new file. "
        "To delete a file, list its path in a top-level `deletions` array (use [] when none). "
        "No commits, pushes, or comments."
    )

def _section(value: dict[str, Any] | str) -> str:
    return value if isinstance(value, str) else compact_json(value)

def build_fix_merge_prompt(premerge_report: dict[str, Any], collection: dict[str, Any], design_plan: dict[str, Any], chief_decision: dict[str, Any], source_context: dict[str, Any] | str) -> str:
    return include_final_patch_contract(
        "Merge conflicting patches.\n"
        f"Premerge: {_section(premerge_report)}\n"
        f"Collection: {_section(collection)}\n"
        f"Design: {_section(design_plan)}\n"
        f"Chief: {_section(chief_decision)}\n"
        f"Source: {_section(source_context)}\n"
    )

def write_fix_merge_prompt(prompt: str, out_path: str | Path) -> Path:
    return write_text(out_path, prompt)
