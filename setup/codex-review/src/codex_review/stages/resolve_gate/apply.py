"""Stage00 lifecycle apply plan and trusted side-effect hooks."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_text
from codex_review.core.constants import TERMINAL_LIFECYCLE_STATES
from codex_review.core.errors import PolicyViolation, ValidationError
from codex_review.github.app_token import assert_installation_token_for_repo, permissions_for_write_mode
from codex_review.github.comments import upsert_sticky_comment
from codex_review.github.issues import build_deferred_issue_body, create_or_update_deferred_issue, make_issue_idempotency_key
from codex_review.github.pull_requests import get_current_head_sha
from codex_review.github.review_threads import reply_to_thread, resolve_thread
from codex_review.loop.state import build_loop_state
from .render import render_resolve_reply, render_sticky_resolve_summary


def apply_resolved_by_code(decision: dict[str, Any], token: str | None = None) -> dict[str, Any]:
    return {"action": "resolve_thread", "thread_id": decision.get("thread_id"), "state": decision.get("state")}


def apply_defer_to_issue(decision: dict[str, Any], token: str | None = None) -> dict[str, Any]:
    return {"action": "defer_to_issue", "thread_id": decision.get("thread_id"), "issue": decision.get("issue") or decision.get("issue_request")}


def apply_duplicate_of_issue(decision: dict[str, Any], token: str | None = None) -> dict[str, Any]:
    return {"action": "mark_duplicate", "thread_id": decision.get("thread_id"), "issue_url": decision.get("issue_url")}


def apply_false_positive_or_stale(decision: dict[str, Any], token: str | None = None) -> dict[str, Any]:
    return {"action": "resolve_thread", "thread_id": decision.get("thread_id"), "state": decision.get("state")}


def leave_non_terminal_open(decision: dict[str, Any]) -> dict[str, Any]:
    return {"action": "leave_open", "thread_id": decision.get("thread_id"), "state": decision.get("state")}


def _plan_action(decision: dict[str, Any]) -> dict[str, Any]:
    state = decision.get("state")
    if state == "resolved_by_code": return apply_resolved_by_code(decision)
    if state == "defer_to_issue": return apply_defer_to_issue(decision)
    if state == "duplicate_of_issue": return apply_duplicate_of_issue(decision)
    if state in {"false_positive", "stale_obsolete"}: return apply_false_positive_or_stale(decision)
    return leave_non_terminal_open(decision)


def _repo_parts(pr_context: dict[str, Any]) -> tuple[str | None, str | None, str | None]:
    owner = pr_context.get("owner")
    repo = pr_context.get("repo")
    repository = pr_context.get("repository")
    if (not owner or not repo) and isinstance(repository, str) and "/" in repository:
        owner, repo = repository.split("/", 1)
    return owner, repo, repository or (f"{owner}/{repo}" if owner and repo else None)


def _verify_apply_allowed(pr_context: dict[str, Any], token: str | None, dry_run: bool) -> None:
    if dry_run:
        return
    if not token:
        raise PolicyViolation("resolve_gate actual apply requires a GitHub App installation token")
    owner, repo, _ = _repo_parts(pr_context)
    if not owner or not repo or not pr_context.get("pr_number"):
        raise ValidationError("resolve_gate actual apply requires owner/repo/pr_number in pr_context")
    assert_installation_token_for_repo(token, owner, repo, permissions_for_write_mode("resolve_gate"))
    expected = pr_context.get("head_sha")
    if expected:
        current = get_current_head_sha(owner, repo, int(pr_context["pr_number"]), token)
        if current and current != expected:
            raise ValidationError(f"head SHA drift before resolve_gate apply: expected {expected}, current {current}")


def apply_lifecycle_result(result: dict[str, Any], pr_context: dict[str, Any], token: str | None, config: dict[str, Any] | None = None, dry_run: bool = True) -> dict[str, Any]:
    _verify_apply_allowed(pr_context, token, dry_run)
    decisions = result.get("decisions") or []
    actions: list[dict[str, Any]] = []
    resolved: list[str] = []
    replies: list[dict[str, Any]] = []
    issues: list[dict[str, Any]] = []
    owner, repo, repository = _repo_parts(pr_context)

    for decision in decisions:
        action = _plan_action(decision)
        actions.append(action)
        state = decision.get("state")
        issue_url = None
        if state == "defer_to_issue":
            issue_req = decision.get("issue") or decision.get("issue_request") or {}
            key = make_issue_idempotency_key(repository or "unknown/unknown", int(pr_context.get("pr_number") or 0), decision.get("root_cause_key") or decision.get("thread_id"))
            planned = {
                "key": key,
                "thread_id": decision.get("thread_id"),
                "title": issue_req.get("title", f"Deferred Codex review: {decision.get('thread_id')}"),
                "body": issue_req.get("body") or build_deferred_issue_body(decision.get("root_cause_key") or decision.get("thread_id"), [decision], pr_context),
            }
            if not dry_run and owner and repo and token:
                planned["result"] = create_or_update_deferred_issue(owner, repo, key, planned["title"], planned["body"], token)
                if isinstance(planned.get("result"), dict):
                    issue_url = planned["result"].get("html_url")
            issues.append(planned)
        if state in TERMINAL_LIFECYCLE_STATES:
            body = render_resolve_reply(decision, issue_url, head_sha=pr_context.get("head_sha"))
            reply = {"thread_id": decision.get("thread_id"), "body": body}
            if not dry_run and token:
                reply["result"] = reply_to_thread(str(decision.get("thread_id")), body, token)
                reply["resolve_result"] = resolve_thread(str(decision.get("thread_id")), token)
            replies.append(reply)
            resolved.append(str(decision.get("thread_id")))

    report: dict[str, Any] = {
        "schema_version": "resolve-gate-apply-report.v1",
        "dry_run": dry_run,
        "actions": actions,
        "resolved_thread_ids": resolved,
        "thread_replies": replies,
        "deferred_issues": issues,
    }
    summary_body = render_sticky_resolve_summary(report, {"route": "resolve_gate_apply"})
    report["sticky_summary"] = {"body": summary_body, "dry_run": dry_run}
    loop_state = build_loop_state("resolve_gate", {"resolved_thread_ids": resolved, "deferred_issue_count": len(issues)}, pr_context.get("head_sha") or "", {"resolve_gate_apply_report": report})
    report["loop_state"] = loop_state
    if not dry_run and owner and repo and token and pr_context.get("pr_number"):
        report["sticky_summary"]["result"] = upsert_sticky_comment(owner, repo, int(pr_context["pr_number"]), "codex-review:resolve-summary", summary_body, token)
    return report


def write_resolve_summary(result: dict[str, Any], apply_report: dict[str, Any], out_path: str | Path) -> Path:
    lines = ["## Stage00 resolve summary", f"Dry run: {apply_report.get('dry_run')}", f"Decisions: {len(result.get('decisions', []))}", f"Resolved: {len(apply_report.get('resolved_thread_ids', []))}", f"Deferred issues: {len(apply_report.get('deferred_issues', []))}"]
    return write_text(out_path, "\n".join(lines) + "\n")
