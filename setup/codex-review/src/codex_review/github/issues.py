"""Deferred issue helpers."""
from __future__ import annotations

import hashlib
import re
import urllib.parse
from typing import Any

from codex_review.core.errors import ValidationError
from .client import api_root, github_api_url, rest_request
from .comments import list_issue_comments
from .markers import render_marker


def make_issue_idempotency_key(repo: str, pr_number: int, root_cause: str | dict[str, Any]) -> str:
    raw = root_cause.get("root_cause_key") if isinstance(root_cause, dict) else root_cause
    material=f"{repo}#{pr_number}:{raw}"
    return hashlib.sha256(material.encode("utf-8")).hexdigest()[:24]


def find_existing_deferred_issue(owner: str, repo: str, key: str, token: str | None) -> dict[str, Any] | None:
    # Search API is best effort; tests can pass mocked rest_request.
    query=f"repo:{owner}/{repo} {key} in:body type:issue"
    url=f"{api_root()}/search/issues?{urllib.parse.urlencode({'q': query})}"
    result=rest_request("GET", url, token)
    items=result.get("items", []) if isinstance(result, dict) else []
    return items[0] if items else None


def build_deferred_issue_body(root_cause: str | dict[str, Any], source_threads: list[dict[str, Any]], pr_context: dict[str, Any]) -> str:
    root = root_cause if isinstance(root_cause, str) else root_cause.get("root_cause_key") or root_cause.get("summary")
    key = make_issue_idempotency_key(pr_context.get("repository") or f"{pr_context.get('owner')}/{pr_context.get('repo')}", int(pr_context.get("pr_number") or 0), str(root))
    lines=[render_marker("codex-review:deferred-issue", {"key": key, "root_cause_key": str(root)}), f"# Deferred Codex review issue", "", f"Root cause: `{root}`", f"Source PR: #{pr_context.get('pr_number')}", "", "## Source threads"]
    for t in source_threads:
        lines.append(f"- {t.get('thread_id') or t.get('id')} {t.get('path') or t.get('file')}:{t.get('line')}")
    return "\n".join(lines)


def create_or_update_deferred_issue(owner: str, repo: str, key: str, title: str, body: str, token: str | None) -> dict[str, Any]:
    existing=find_existing_deferred_issue(owner, repo, key, token)
    if existing:
        return rest_request("PATCH", existing["url"], token, {"title": title, "body": body})
    return rest_request("POST", github_api_url(owner, repo, "/issues"), token, {"title": title, "body": body})


def validate_same_repo_issue_url(url: str, owner: str, repo: str) -> None:
    import os
    server = os.environ.get("GITHUB_SERVER_URL", "https://github.com").rstrip("/")
    pattern = rf"^{re.escape(server)}/{re.escape(owner)}/{re.escape(repo)}/issues/\d+$"
    if not re.match(pattern, url or ""):
        raise ValidationError(f"issue URL is not in the same repository: {url}")
