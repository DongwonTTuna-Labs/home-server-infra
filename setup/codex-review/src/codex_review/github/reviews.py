"""Pull request review helpers."""
from __future__ import annotations

from collections import defaultdict
from typing import Any

from codex_review.context.diff import is_changed_right_line
from codex_review.core.errors import ValidationError
from .client import github_api_url, rest_request


def validate_inline_comment_position(file: str, line: int, changed_lines: dict[str, Any]) -> None:
    if not is_changed_right_line(changed_lines, file, line):
        raise ValidationError(f"inline comment target is not a changed RIGHT-side line: {file}:{line}")


def build_inline_comment_payload(decision: dict[str, Any], changed_lines: dict[str, Any]) -> dict[str, Any]:
    file=decision.get("file") or decision.get("path")
    line=int(decision.get("line") or 0)
    validate_inline_comment_position(file, line, changed_lines)
    return {"path": file, "line": line, "side": "RIGHT", "body": decision.get("body") or decision.get("comment") or decision.get("summary") or "Codex review finding"}


def limit_inline_comments(comments: list[dict[str, Any]], policy: dict[str, Any]) -> list[dict[str, Any]]:
    max_total=int(policy.get("max_inline_comments", 12))
    max_per_file=int(policy.get("max_inline_comments_per_file", 3))
    counts=defaultdict(int); out=[]
    for c in comments:
        path=c.get("path")
        if len(out) >= max_total:
            break
        if counts[path] >= max_per_file:
            continue
        counts[path]+=1; out.append(c)
    return out


def create_pull_request_review(owner: str, repo: str, pr_number: int, comments: list[dict[str, Any]], body: str, event: str, token: str | None, commit_id: str | None = None) -> dict[str, Any]:
    payload = {"body": body, "event": event, "comments": comments}
    if commit_id:
        payload["commit_id"] = commit_id
    return rest_request("POST", github_api_url(owner, repo, f"/pulls/{pr_number}/reviews"), token, payload)
