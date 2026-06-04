"""Repository documentation context helpers."""
from __future__ import annotations

from pathlib import Path
from typing import Any

DOC_CANDIDATES = ["AGENTS.md", "TESTING.md", "REVIEW_CHECKLIST.md", "README.md", "CONTRIBUTING.md", "docs/README.md"]


def find_repository_docs(root: str | Path) -> list[Path]:
    base = Path(root)
    return [base / name for name in DOC_CANDIDATES if (base / name).exists()]


def read_docs_with_budget(paths: list[str | Path], budget: int = 20000) -> list[dict[str, Any]]:
    docs=[]; remaining=budget
    for path in paths:
        p=Path(path)
        if not p.exists() or remaining <= 0:
            continue
        text=p.read_text(encoding="utf-8", errors="replace")
        if len(text) > remaining:
            text=text[:remaining] + "\n...[truncated]"
        remaining -= len(text)
        docs.append({"path": p.as_posix(), "text": text})
    return docs


def classify_doc_relevance(path: str | Path, stage: str) -> str:
    name=Path(path).name.lower()
    if "test" in name:
        return "tests"
    if "agent" in name or "review" in name:
        return "workflow-rules"
    if stage.startswith("fix_dispatch") or stage.startswith("push"):
        return "autofix"
    return "general"


def render_docs_context(docs: list[dict[str, Any]]) -> str:
    if not docs:
        return "## Repository docs\n\nNo repository docs were found.\n"
    parts=["## Repository docs"]
    for doc in docs:
        parts.append(f"\n### {doc.get('path')}\n\n```text\n{doc.get('text','')}\n```")
    return "\n".join(parts)
