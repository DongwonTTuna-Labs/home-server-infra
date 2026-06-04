"""Review thread GraphQL helpers."""
from __future__ import annotations

from typing import Any

from .client import graphql_request
from .graphql_fragments import reply_to_thread_mutation, resolve_thread_mutation, review_threads_query


def normalize_thread_node(node: dict[str, Any]) -> dict[str, Any]:
    raw_comments = node.get("comments") or []
    if isinstance(raw_comments, dict):
        comments = raw_comments.get("nodes") or []
    else:
        comments = raw_comments
    normalized=[]
    for c in comments:
        normalized.append({
            "id": c.get("id"),
            "body": c.get("body") or "",
            "author": (c.get("author") or {}).get("login") if isinstance(c.get("author"), dict) else c.get("author"),
            "url": c.get("url"),
            "path": c.get("path") or node.get("path"),
            "line": c.get("line") or node.get("line"),
            "commit_id": (c.get("commit") or {}).get("oid") or c.get("commit_id"),
            "original_commit_id": (c.get("originalCommit") or {}).get("oid") or c.get("original_commit_id"),
            "created_at": c.get("createdAt") or c.get("created_at"),
            "updated_at": c.get("updatedAt") or c.get("updated_at"),
        })
    return {
        "id": node.get("id") or node.get("thread_id"),
        "thread_id": node.get("id") or node.get("thread_id"),
        "isResolved": bool(node.get("isResolved") or node.get("resolved") or node.get("is_resolved")),
        "path": node.get("path"),
        "line": node.get("line") or node.get("originalLine"),
        "start_line": node.get("startLine"),
        "comments": normalized,
    }


def collect_review_threads(owner: str, repo: str, pr_number: int, token: str | None) -> list[dict[str, Any]]:
    threads=[]; cursor=None
    while True:
        data=graphql_request(review_threads_query(), {"owner": owner, "repo": repo, "number": int(pr_number), "cursor": cursor}, token)
        rt = (((data.get("repository") or {}).get("pullRequest") or {}).get("reviewThreads") or {})
        nodes=rt.get("nodes") or []
        threads.extend(normalize_thread_node(n) for n in nodes)
        page=rt.get("pageInfo") or {}
        if not page.get("hasNextPage"):
            break
        cursor=page.get("endCursor")
    return threads


def reply_to_thread(thread_id: str, body: str, token: str | None) -> dict[str, Any]:
    return graphql_request(reply_to_thread_mutation(), {"threadId": thread_id, "body": body}, token)


def resolve_thread(thread_id: str, token: str | None) -> dict[str, Any]:
    return graphql_request(resolve_thread_mutation(), {"threadId": thread_id}, token)


def is_thread_resolved(thread: dict[str, Any]) -> bool:
    return bool(thread.get("isResolved") or thread.get("resolved") or thread.get("is_resolved"))
