"""Unified-diff parsing helpers."""
from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from codex_review.memory.paths import is_memory_path

HUNK_RE = re.compile(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")


def _strip_prefix(path: str) -> str:
    return path[2:] if path.startswith(("a/", "b/")) else path


def parse_unified_diff(diff_text: str) -> list[dict[str, Any]]:
    files: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    hunk: dict[str, Any] | None = None
    old_line = new_line = 0
    pending_file: str | None = None
    for raw in diff_text.splitlines():
        line = raw.rstrip("\n")
        if line.startswith("diff --git "):
            parts = line.split()
            new_path = _strip_prefix(parts[3]) if len(parts) >= 4 else None
            current = {"old_path": _strip_prefix(parts[2]) if len(parts) >= 3 else new_path, "new_path": new_path, "hunks": []}
            files.append(current)
            hunk = None
            pending_file = new_path
            continue
        if line.startswith("+++ "):
            path = line[4:].strip()
            if path != "/dev/null":
                pending_file = _strip_prefix(path)
                if current is None:
                    current = {"old_path": None, "new_path": pending_file, "hunks": []}
                    files.append(current)
                current["new_path"] = pending_file
            continue
        m = HUNK_RE.match(line)
        if m:
            if current is None:
                current = {"old_path": None, "new_path": pending_file or "<unknown>", "hunks": []}
                files.append(current)
            old_line = int(m.group(1)); new_line = int(m.group(3))
            hunk = {"header": line, "old_start": old_line, "new_start": new_line, "lines": []}
            current["hunks"].append(hunk)
            continue
        if hunk is None:
            continue
        if line.startswith("+") and not line.startswith("+++"):
            hunk["lines"].append({"kind": "add", "old_line": None, "new_line": new_line, "text": line[1:]})
            new_line += 1
        elif line.startswith("-") and not line.startswith("---"):
            hunk["lines"].append({"kind": "del", "old_line": old_line, "new_line": None, "text": line[1:]})
            old_line += 1
        else:
            text = line[1:] if line.startswith(" ") else line
            hunk["lines"].append({"kind": "context", "old_line": old_line, "new_line": new_line, "text": text})
            old_line += 1; new_line += 1
    return files


def extract_changed_right_lines(diff: list[dict[str, Any]] | str) -> dict[str, set[int]]:
    parsed = parse_unified_diff(diff) if isinstance(diff, str) else diff
    result: dict[str, set[int]] = {}
    for file in parsed:
        path = file.get("new_path") or file.get("path") or file.get("filename")
        if not path or path == "/dev/null":
            continue
        path = str(path)
        if is_memory_path(path):
            continue
        lines: set[int] = result.setdefault(path, set())
        for hunk in file.get("hunks", []):
            for line in hunk.get("lines", []):
                if line.get("kind") == "add" and line.get("new_line") is not None:
                    lines.add(int(line["new_line"]))
    return result


def summarize_diff(diff: list[dict[str, Any]] | str, max_chars: int = 12000) -> str:
    text = diff if isinstance(diff, str) else "\n".join(
        f"{f.get('new_path')}: {sum(1 for h in f.get('hunks', []) for l in h.get('lines', []) if l.get('kind') == 'add')} added lines"
        for f in diff
    )
    if len(text) > max_chars:
        return text[: max_chars - 40] + "\n...[diff truncated]"
    return text


def hunk_headers(patch_text: str) -> str:
    """Return only the ``@@ ... @@`` hunk headers from a unified-diff patch.

    Used when a per-file patch exceeds its token budget: the headers preserve which
    line ranges changed (so a reviewer still knows where to look) while dropping the
    body that would blow the context window.
    """
    return "\n".join(line for line in (patch_text or "").splitlines() if line.startswith("@@"))


def find_context_window(file_path: str | Path, line: int, radius: int = 8) -> dict[str, Any]:
    p = Path(file_path)
    if not p.exists():
        return {"file": str(file_path), "line": line, "start": None, "end": None, "text": ""}
    lines = p.read_text(encoding="utf-8", errors="replace").splitlines()
    start = max(1, line - radius)
    end = min(len(lines), line + radius)
    window = "\n".join(f"{idx}: {lines[idx-1]}" for idx in range(start, end + 1))
    return {"file": str(file_path), "line": line, "start": start, "end": end, "text": window}


def _filter_memory_paths(changed_map: dict[str, set[int]]) -> dict[str, set[int]]:
    return {path: lines for path, lines in changed_map.items() if not is_memory_path(path)}


# --- Changed RIGHT-side line map helpers -------------------------------------


def _normalize_lines(lines: Any) -> set[int]:
    if isinstance(lines, set):
        return {int(x) for x in lines}
    if isinstance(lines, list):
        return {int(x) for x in lines}
    return set()


def build_changed_line_map(pr_files: list[dict[str, Any]] | dict[str, Any] | str) -> dict[str, set[int]]:
    if isinstance(pr_files, str):
        return _filter_memory_paths(extract_changed_right_lines(pr_files))
    if isinstance(pr_files, dict):
        if "changed_lines" in pr_files:
            return _filter_memory_paths({str(k): _normalize_lines(v) for k, v in pr_files["changed_lines"].items()})
        pr_files = pr_files.get("files") or pr_files.get("changed_files") or []
    changed: dict[str, set[int]] = {}
    for f in pr_files:
        filename = f.get("filename") or f.get("path") or f.get("new_path")
        patch = f.get("patch") or f.get("diff") or ""
        if not filename:
            continue
        if is_memory_path(str(filename)):
            continue
        if patch and not patch.startswith("diff --git"):
            patch = f"diff --git a/{filename} b/{filename}\n--- a/{filename}\n+++ b/{filename}\n{patch}"
        if patch:
            extracted = extract_changed_right_lines(parse_unified_diff(patch))
            lines = extracted.get(str(filename), set())
            if not lines and len(extracted) == 1:
                lines = next(iter(extracted.values()))
        else:
            lines = set()
        if not lines and f.get("changed_lines"):
            lines = _normalize_lines(f["changed_lines"])
        changed[str(filename)] = lines
    return changed


def is_changed_right_line(changed_map: dict[str, Any], file: str, line: int | str) -> bool:
    try:
        n = int(line)
    except Exception:
        return False
    return n in _normalize_lines(changed_map.get(file, set()))


def nearest_changed_line(changed_map: dict[str, Any], file: str, line: int | str) -> int | None:
    lines = sorted(_normalize_lines(changed_map.get(file, set())))
    if not lines:
        return None
    try:
        n = int(line)
    except Exception:
        return lines[0]
    return min(lines, key=lambda x: (abs(x - n), x))


def serialize_changed_line_map(changed_map: dict[str, Any]) -> dict[str, list[int]]:
    return {path: sorted(_normalize_lines(lines)) for path, lines in changed_map.items()}
