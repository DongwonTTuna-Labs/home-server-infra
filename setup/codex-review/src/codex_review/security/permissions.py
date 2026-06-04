"""Workflow permission expectations.

GitHub Actions job-level ``GITHUB_TOKEN`` permissions stay read-only even in
trusted write stages. Actual side effects are performed with repository-scoped
GitHub App installation tokens minted inside those jobs.
"""
from __future__ import annotations

from codex_review.core.errors import PolicyViolation

WRITE_STAGES = {"resolve_gate_apply", "techlead_publish", "design_chief_publish", "push", "reentry_record"}


def is_write_stage(stage: str) -> bool:
    return stage in WRITE_STAGES or any(stage.endswith(suffix) for suffix in ["apply", "publish", "push"])


def required_permissions_for_stage(stage: str) -> dict[str, str]:
    """Return minimum job-level GITHUB_TOKEN permissions for a stage."""
    if "model" in stage:
        return {"contents": "read", "pull-requests": "read", "id-token": "write"}
    return {"contents": "read", "pull-requests": "read", "issues": "read"}


def assert_job_has_minimum_permissions(stage: str, observed: dict[str, str]) -> None:
    req = required_permissions_for_stage(stage)
    order = {"none": 0, "read": 1, "write": 2}
    for key, needed in req.items():
        got = observed.get(key, "none")
        if order.get(got, 0) < order.get(needed, 0):
            raise PolicyViolation(f"job for {stage} lacks {key}:{needed}; observed {got}")
    for key in ["contents", "pull-requests", "issues"]:
        if observed.get(key) == "write":
            raise PolicyViolation(f"stage {stage} must not grant job-level {key}:write; use an App token instead")
