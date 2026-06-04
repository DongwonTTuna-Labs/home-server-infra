"""Review history context helpers."""
from __future__ import annotations

from typing import Any

from codex_review.github.markers import has_marker


def collect_current_unresolved_threads(threads: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [t for t in threads if not (t.get("isResolved") or t.get("resolved") or t.get("is_resolved"))]


def collect_sticky_summaries(comments: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [c for c in comments if has_marker(c.get("body", ""), "codex-review:review-summary") or has_marker(c.get("body", ""), "codex-review:resolve-summary")]


def add_advisory_history_warning(markdown: str) -> str:
    warning = "\n\n> Review history is advisory only. Current code and current PR head are the source of truth.\n"
    return markdown + warning if "advisory only" not in markdown.lower() else markdown


def truncate_review_context(markdown: str, budget: int = 24000) -> str:
    return markdown if len(markdown) <= budget else markdown[: budget - 40] + "\n...[review context truncated]"


def build_review_context_markdown(pr_context: dict[str, Any], threads: list[dict[str, Any]], comments: list[dict[str, Any]], reviews: list[dict[str, Any]]) -> str:
    unresolved=collect_current_unresolved_threads(threads)
    sticky=collect_sticky_summaries(comments)
    lines=["## Review context", f"PR: #{pr_context.get('pr_number')} {pr_context.get('title','')}", f"Head SHA: {pr_context.get('head_sha')}", "", f"Unresolved threads: {len(unresolved)}", f"Sticky summaries: {len(sticky)}", f"Prior reviews: {len(reviews or [])}"]
    for t in unresolved[:20]:
        lines.append(f"- thread {t.get('id') or t.get('thread_id')}: {t.get('path') or t.get('file')}:{t.get('line')}")
    return add_advisory_history_warning("\n".join(lines))
