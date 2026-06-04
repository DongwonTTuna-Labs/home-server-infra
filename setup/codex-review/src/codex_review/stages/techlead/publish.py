"""Stage02 trusted review posting."""
from __future__ import annotations
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json
from codex_review.github.comments import upsert_sticky_comment
from codex_review.core.errors import ValidationError
from codex_review.github.app_token import assert_installation_token_for_repo, permissions_for_write_mode
from codex_review.github.pull_requests import get_current_head_sha
from codex_review.github.reviews import create_pull_request_review, limit_inline_comments, validate_inline_comment_position
from .render import render_inline_comment, render_review_body, render_sticky_review_summary


def build_inline_comments(publication: dict[str, Any], changed_line_map: dict[str, Any], config: dict[str, Any]) -> list[dict[str, Any]]:
    comments=[]
    for item in publication.get("inline_comments", []):
        file=item.get("file") or item.get("path"); line=int(item.get("line") or 0)
        validate_inline_comment_position(file, line, changed_line_map)
        comments.append({"path": file, "line": line, "side": "RIGHT", "body": render_inline_comment(item)})
    return limit_inline_comments(comments, config.get("review", {}))


def upsert_review_summary(publication: dict[str, Any], token: str | None, pr_context: dict[str, Any] | None = None) -> dict[str, Any]:
    if not token or not pr_context:
        return {"dry_run": True, "body": render_sticky_review_summary(publication)}
    return upsert_sticky_comment(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), "codex-review:review-summary", render_sticky_review_summary(publication), token)


def _verify_publish_allowed(pr_context: dict[str, Any], token: str | None, dry_run: bool) -> str | None:
    if dry_run:
        return pr_context.get("head_sha")
    missing = [key for key in ["owner", "repo", "pr_number"] if not pr_context.get(key)]
    if missing:
        raise ValidationError(f"techlead publish requires pr_context fields: {', '.join(missing)}")
    assert_installation_token_for_repo(token, pr_context["owner"], pr_context["repo"], permissions_for_write_mode("techlead"))
    expected = pr_context.get("head_sha")
    current = get_current_head_sha(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), token)
    if expected and current and expected != current:
        raise ValidationError(f"head SHA drift before techlead publish: expected {expected}, current {current}")
    return current or expected


def publish_review(publication: dict[str, Any], pr_context: dict[str, Any], changed_line_map: dict[str, Any], token: str | None, config: dict[str, Any], dry_run: bool = False) -> dict[str, Any]:
    comments=build_inline_comments(publication, changed_line_map, config)
    body=render_review_body(publication)
    effective_token = None if dry_run else token
    commit_id = _verify_publish_allowed(pr_context, effective_token, dry_run)
    report={"schema_version":"techlead-publish-report.v1","inline_comment_count":len(comments),"dry_run": dry_run,"comments":comments,"body":body,"commit_id": commit_id}
    if effective_token and comments:
        report["review"]=create_pull_request_review(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), comments, body, "COMMENT", effective_token, commit_id=commit_id)
    report["summary"]=upsert_review_summary(publication, effective_token, pr_context if effective_token else None)
    return report


def write_publish_report(report: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, report, "techlead-publish-report.v1")
