"""Issue comment helpers."""
from __future__ import annotations

from typing import Any

from .client import github_api_url, rest_paginated, rest_request
from .markers import has_marker


def list_issue_comments(owner: str, repo: str, issue_number: int, token: str | None) -> list[dict[str, Any]]:
    return rest_paginated(github_api_url(owner, repo, f"/issues/{issue_number}/comments"), token, {"per_page": 100})


def find_sticky_comment(comments: list[dict[str, Any]], marker: str) -> dict[str, Any] | None:
    matches=[c for c in comments if has_marker(c.get("body", ""), marker)]
    if not matches:
        return None
    return sorted(matches, key=lambda c: c.get("updated_at") or c.get("created_at") or "")[-1]


def upsert_sticky_comment(owner: str, repo: str, issue_number: int, marker: str, body: str, token: str | None) -> dict[str, Any]:
    comments=list_issue_comments(owner, repo, issue_number, token)
    existing=find_sticky_comment(comments, marker)
    full_body = body if marker in body else f"{body}\n\n<!-- {marker} -->"
    if existing:
        return rest_request("PATCH", existing["url"], token, {"body": full_body})
    return rest_request("POST", github_api_url(owner, repo, f"/issues/{issue_number}/comments"), token, {"body": full_body})


def delete_or_archive_comment_if_needed(comment_id: int | str, policy: dict[str, Any]) -> dict[str, Any]:
    if policy.get("delete_stale_comments"):
        return {"action": "delete", "comment_id": comment_id}
    return {"action": "archive", "comment_id": comment_id}
