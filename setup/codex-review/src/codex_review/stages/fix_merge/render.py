"""Stage06 render helpers."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_text
from codex_review.security.patch_policy import parse_patch_touched_files

def render_premerge_summary(report: dict[str, Any]) -> str:
    return f"## Premerge\n\nClean: {report.get('clean')}\nResults: {len(report.get('results', []))}\n"

def render_merged_fix_summary(merged_fix: dict[str, Any]) -> str:
    touched=parse_patch_touched_files(merged_fix.get("patch") or merged_fix.get("patch_text") or "")
    return f"## Merged fix\n\nTouched files: {', '.join(touched)}\n"

def write_diffstat(patch_text: str, out_path: str | Path) -> Path:
    touched=parse_patch_touched_files(patch_text)
    return write_text(out_path, "\n".join(touched)+"\n")
