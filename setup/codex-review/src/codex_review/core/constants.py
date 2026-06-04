"""Constants and schema-version helpers for Codex Review v3."""
from __future__ import annotations

STAGES = [
    "resolve_gate",
    "review",
    "techlead",
    "design",
    "design_chief",
    "fix_dispatch",
    "fix_merge",
    "push",
    "reentry",
]

MARKERS = {
    "inline_review": "codex-review:inline",
    "lifecycle": "codex-review:lifecycle",
    "sticky_review_summary": "codex-review:review-summary",
    "sticky_resolve_summary": "codex-review:resolve-summary",
    "loop_state": "codex-review:loop-state",
    "deferred_issue": "codex-review:deferred-issue",
    "design_summary": "codex-review:design-summary",
}

_SCHEMA_BY_ARTIFACT = {
    "event-context": "event-context.v1",
    "pr-context": "pr-context.v1",
    "loop-state": "loop-state.v1",
    "shared-review-finding": "shared-review-finding.v1",
    "resolve-gate-thread-inventory": "resolve-gate-thread-inventory.v1",
    "resolve-gate-lifecycle-result": "resolve-gate-lifecycle-result.v1",
    "resolve-gate-result": "resolve-gate-result.v1",
    "review-axis-findings": "review-axis-findings.v1",
    "review-combined-findings": "review-combined-findings.v1",
    "techlead-decision": "techlead-decision.v1",
    "techlead-review-publication": "techlead-review-publication.v1",
    "design-context": "design-context.v1",
    "design-inventory": "design-inventory.v1",
    "design-clusters": "design-clusters.v1",
    "design-cluster-analysis": "design-cluster-analysis.v1",
    "design-plan": "design-plan.v1",
    "design-chief-decision": "design-chief-decision.v1",
    "fix-dispatch-task-manifest": "fix-dispatch-task-manifest.v1",
    "fix-dispatch-agent-result": "fix-dispatch-agent-result.v1",
    "fix-dispatch-collection-result": "fix-dispatch-collection-result.v1",
    "fix-merge-premerge-report": "fix-merge-premerge-report.v1",
    "fix-merge-merged-fix": "fix-merge-merged-fix.v1",
    "push-result": "push-result.v1",
    "reentry-loop-state": "reentry-loop-state.v1",
}

TERMINAL_LIFECYCLE_STATES = {
    "resolved_by_code",
    "defer_to_issue",
    "duplicate_of_issue",
    "false_positive",
    "stale_obsolete",
}
NON_TERMINAL_LIFECYCLE_STATES = {"fix_now", "current_head_keep_open", "needs_human", "blocked_by_conflict"}
LIFECYCLE_STATES = TERMINAL_LIFECYCLE_STATES | NON_TERMINAL_LIFECYCLE_STATES

TECHLEAD_ACTIONS = {
    "publish_and_fix_now",
    "summary_only_fix_now",
    "defer_to_issue",
    "deny_false_positive",
    "needs_human",
    "needs_design",
    "publish_only",
    "drop_duplicate",
}

SEVERITIES = {"critical", "high", "medium", "low", "info"}


def schema_version_for(artifact_name: str) -> str:
    key = artifact_name.removesuffix(".json").removesuffix(".schema")
    key = key.removesuffix(".v1")
    if key.endswith(".v1.schema"):
        key = key[:-10]
    if key in _SCHEMA_BY_ARTIFACT:
        return _SCHEMA_BY_ARTIFACT[key]
    # Allow callers to pass the exact version string.
    if key.endswith(".v1"):
        return key
    raise KeyError(f"unknown artifact schema version: {artifact_name}")


def marker_names() -> dict[str, str]:
    return dict(MARKERS)


def stage_names() -> list[str]:
    return list(STAGES)
