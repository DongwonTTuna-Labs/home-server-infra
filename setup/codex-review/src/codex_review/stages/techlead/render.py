"""Stage02 rendering."""
from __future__ import annotations
from typing import Any
from codex_review.github.markers import render_inline_review_marker, render_marker


def render_inline_comment(decision_item: dict[str, Any], finding: dict[str, Any] | None = None) -> str:
    f={**(finding or {}), **decision_item}
    body=f"**{f.get('title','Codex review finding')}**\n\n{f.get('summary','')}\n\nRecommendation: {f.get('recommendation') or f.get('reason','Review this finding.')}"
    return body + "\n\n" + render_inline_review_marker(str(f.get("finding_id")), str(f.get("root_cause_key") or f.get("finding_id")))


def render_review_body(publication: dict[str, Any]) -> str:
    return f"Codex Review summary\n\nStatus: `{publication.get('status')}`\nInline comments: {len(publication.get('inline_comments', []))}\nSummary items: {len(publication.get('summary_items', []))}\n"


def render_sticky_review_summary(publication: dict[str, Any]) -> str:
    return render_review_body(publication) + "\n" + render_marker("codex-review:review-summary", {"status": publication.get("status")})


def render_deferred_summary(publication: dict[str, Any]) -> str:
    lines=["## Deferred findings"]
    for item in publication.get("deferred_items", []): lines.append(f"- {item.get('finding_id')}: {item.get('summary')}")
    return "\n".join(lines)
