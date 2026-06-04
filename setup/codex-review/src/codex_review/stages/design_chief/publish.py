"""Publish design chief summary."""
from __future__ import annotations
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError
from codex_review.github.app_token import assert_installation_token_for_repo, permissions_for_write_mode
from codex_review.github.pull_requests import get_current_head_sha
from codex_review.github.comments import upsert_sticky_comment
from codex_review.github.markers import render_marker
from codex_review.stages.design.render import render_design_plan_markdown
from .render import render_chief_decision_markdown


def render_design_chief_summary(plan: dict[str, Any], chief_decision: dict[str, Any]) -> str:
    return render_design_plan_markdown(plan) + "\n" + render_chief_decision_markdown(chief_decision) + "\n" + render_marker("codex-review:design-summary", {"status": chief_decision.get("status"), "plan_hash": plan.get("plan_hash")})


def _verify_design_publish_allowed(pr_context: dict[str, Any] | None, token: str | None, dry_run: bool) -> None:
    if dry_run:
        return
    if not pr_context:
        raise ValidationError("design_chief publish requires pr_context")
    missing = [key for key in ["owner", "repo", "pr_number"] if not pr_context.get(key)]
    if missing:
        raise ValidationError(f"design_chief publish requires pr_context fields: {', '.join(missing)}")
    assert_installation_token_for_repo(token, pr_context["owner"], pr_context["repo"], permissions_for_write_mode("design_chief"))
    expected = pr_context.get("head_sha")
    current = get_current_head_sha(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), token)
    if expected and current and expected != current:
        raise ValidationError(f"head SHA drift before design_chief publish: expected {expected}, current {current}")


def publish_design_summary(plan: dict[str, Any], chief_decision: dict[str, Any], token: str | None = None, pr_context: dict[str, Any] | None = None, dry_run: bool = False) -> dict[str, Any]:
    body = render_design_chief_summary(plan, chief_decision)
    effective_token = None if dry_run else token
    _verify_design_publish_allowed(pr_context, effective_token, dry_run)
    report = {"schema_version": "design-chief-publish-report.v1", "dry_run": dry_run, "body": body}
    if effective_token and pr_context and pr_context.get("owner") and pr_context.get("repo") and pr_context.get("pr_number"):
        report["summary"] = upsert_sticky_comment(pr_context["owner"], pr_context["repo"], int(pr_context["pr_number"]), "codex-review:design-summary", body, effective_token)
    return report


def write_design_publish_report(report: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, report, "design-chief-publish-report.v1")
