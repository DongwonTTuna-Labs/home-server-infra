"""Pull request REST helpers."""
from __future__ import annotations

from typing import Any

from codex_review.core.errors import ValidationError
from .client import github_api_url, rest_paginated, rest_request


def get_pull_request(owner: str, repo: str, pr_number: int, token: str | None) -> dict[str, Any]:
    return rest_request("GET", github_api_url(owner, repo, f"/pulls/{pr_number}"), token)


def list_pull_request_files(owner: str, repo: str, pr_number: int, token: str | None) -> list[dict[str, Any]]:
    return rest_paginated(github_api_url(owner, repo, f"/pulls/{pr_number}/files"), token, {"per_page": 100})


def list_pull_request_commits(owner: str, repo: str, pr_number: int, token: str | None) -> list[dict[str, Any]]:
    return rest_paginated(github_api_url(owner, repo, f"/pulls/{pr_number}/commits"), token, {"per_page": 100})


def get_current_head_sha(owner: str, repo: str, pr_number: int, token: str | None) -> str:
    pr = get_pull_request(owner, repo, pr_number, token)
    return pr.get("head", {}).get("sha")


def validate_pr_is_open(pr: dict[str, Any]) -> None:
    if pr.get("state") != "open":
        raise ValidationError(f"PR must be open, got {pr.get('state')!r}")


def validate_same_repo_or_trusted(pr: dict[str, Any], policy: dict[str, Any]) -> None:
    head=(pr.get("head") or {}).get("repo") or {}; base=(pr.get("base") or {}).get("repo") or {}
    same = head.get("full_name") == base.get("full_name")
    if same:
        return
    allowed = set(policy.get("trusted_fork_owners") or [])
    owner = (head.get("owner") or {}).get("login")
    if owner not in allowed:
        raise ValidationError(f"untrusted fork PR head repository: {head.get('full_name')}")
