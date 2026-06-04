"""Stage00 Markdown rendering."""
from __future__ import annotations

from typing import Any

from codex_review.github.markers import render_marker


def render_resolve_reply(decision: dict[str, Any], issue_url: str | None = None, head_sha: str | None = None) -> str:
    state=decision.get("state")
    evidence=decision.get("evidence") or decision.get("reason") or "Validated by Codex Review lifecycle gate."
    lines=[f"Codex Review lifecycle decision: `{state}`", "", str(evidence)]
    if issue_url:
        lines += ["", f"Deferred follow-up issue: {issue_url}"]
    # Machine-readable marker so future runs can recover the resolution STATE
    # (not just prose) and suppress re-flagged findings. root_cause_key is also
    # recoverable from the thread's original inline marker during harvest.
    marker_payload={"state": state, "head_sha": head_sha or decision.get("head_sha"), "root_cause_key": decision.get("root_cause_key")}
    lines += ["", render_marker("codex-review:resolved", {k: v for k, v in marker_payload.items() if v})]
    return "\n".join(lines)


def render_sticky_resolve_summary(apply_report: dict[str, Any], gate_result: dict[str, Any]) -> str:
    return "\n".join(["## Codex resolve gate", f"Route: `{gate_result.get('route')}`", f"Resolved threads: {len(apply_report.get('resolved_thread_ids', []))}", f"Deferred issues: {len(apply_report.get('deferred_issues', []))}"])


def render_resolve_gate_step_summary(gate_result: dict[str, Any]) -> str:
    return f"## Stage00\n\nRoute: `{gate_result.get('route')}`\nThreads: {gate_result.get('thread_count', 0)}\n"


def render_needs_human_reason(decisions: list[dict[str, Any]]) -> str:
    reasons=[f"- {d.get('thread_id')}: {d.get('reason') or d.get('evidence') or 'needs human review'}" for d in decisions if d.get("state") == "needs_human"]
    return "## Needs human review\n" + "\n".join(reasons)
