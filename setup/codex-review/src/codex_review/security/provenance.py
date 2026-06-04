"""Actor, PR and commit provenance checks."""
from __future__ import annotations

from typing import Any

from codex_review.core.errors import PolicyViolation


def is_trusted_codex_review_author(author: str | dict[str, Any] | None, policy: dict[str, Any]) -> bool:
    login = author.get("login") if isinstance(author, dict) else author
    trusted=set(policy.get("codex_review_authors") or policy.get("trusted", {}).get("codex_review_authors") or [])
    return bool(login and login in trusted)


def is_trusted_workflow_actor(actor: str | None, triggering_actor: str | None, sender: str | dict[str, Any] | None, policy: dict[str, Any]) -> bool:
    sender_login=sender.get("login") if isinstance(sender, dict) else sender
    allowed=set([policy.get("user"), policy.get("trusted", {}).get("user"), *(policy.get("trusted_actors") or [])])
    allowed={x for x in allowed if x}
    return any(x in allowed for x in [actor, triggering_actor, sender_login])


def validate_pull_request_target_safety(event: dict[str, Any], pr: dict[str, Any], policy: dict[str, Any]) -> dict[str, Any]:
    head=(pr.get("head") or event.get("pull_request", {}).get("head") or {})
    base=(pr.get("base") or event.get("pull_request", {}).get("base") or {})
    same_repo=(head.get("repo") or {}).get("full_name") == (base.get("repo") or {}).get("full_name")
    actor=event.get("actor") or (event.get("sender") or {}).get("login")
    trusted_actor=is_trusted_workflow_actor(actor, event.get("triggering_actor"), event.get("sender"), policy)
    if not same_repo and not trusted_actor:
        raise PolicyViolation("pull_request_target write side effects are blocked for untrusted fork PR")
    return {"same_repo": same_repo, "trusted_actor": trusted_actor, "safe_for_write": same_repo and trusted_actor}


def is_codex_autofix_commit(commit: dict[str, Any], policy: dict[str, Any] | None = None) -> bool:
    msg=(commit.get("commit", {}) or {}).get("message") or commit.get("message") or ""
    author=((commit.get("commit", {}) or {}).get("author") or {}).get("name") or commit.get("author", {}).get("login")
    return "codex-review:autofix" in msg or "Codex Review Autofix" in msg or str(author).lower().startswith("codex")


def count_existing_codex_autofix_commits(commits: list[dict[str, Any]], policy: dict[str, Any] | None = None) -> int:
    return sum(1 for commit in commits or [] if is_codex_autofix_commit(commit, policy or {}))
