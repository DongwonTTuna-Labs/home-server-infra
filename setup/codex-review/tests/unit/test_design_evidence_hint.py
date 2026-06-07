"""The design plan/chief prompts must steer models to cite existing files.

A live loop failed because the chief model cited the to-be-created file as
inspection_evidence (which the validator rejects). These tests lock in that the
prompts surface the upstream-verified existing paths as candidate citations plus
an explicit DON'T example.
"""
from codex_review.model.inspection import (
    collect_existing_evidence_paths,
    render_evidence_citation_hint,
)
from codex_review.stages.design.coordinate import build_coordinate_prompt
from codex_review.stages.design_chief.prompt import build_design_chief_prompt


def _payload(*paths):
    return {
        "inspection_evidence": [
            {"path": p, "purpose": "x", "observation": "y"} for p in paths
        ]
    }


def test_collect_existing_evidence_paths_dedupes_across_payloads():
    a = _payload("openspec/changes/foo/spec.md", "tasks.md")
    b = _payload("tasks.md", "design.md")
    assert collect_existing_evidence_paths(a, b, {}, None) == [
        "openspec/changes/foo/spec.md",
        "tasks.md",
        "design.md",
    ]


def test_citation_hint_always_forbids_nonexistent_and_shows_example():
    hint = render_evidence_citation_hint([])
    assert "NEVER cite" in hint
    assert "WRONG:" in hint and "CORRECT:" in hint
    # No candidate list when there are no known existing paths.
    assert "already verified to exist" not in hint


def test_citation_hint_lists_candidate_paths_when_present():
    hint = render_evidence_citation_hint(["openspec/changes/foo/spec.md"])
    assert "already verified to exist: openspec/changes/foo/spec.md" in hint


def test_plan_prompt_carries_inventory_paths_and_dont_rule():
    inventory = _payload("openspec/changes/foo/tasks.md")  # passed as `clusters`
    prompt = build_coordinate_prompt({"findings": [{"finding_id": "f1"}]}, inventory, [])
    assert "openspec/changes/foo/tasks.md" in prompt
    assert "NEVER cite" in prompt and "WRONG:" in prompt


def test_chief_prompt_carries_plan_and_techlead_paths_and_dont_rule():
    plan = _payload("openspec/changes/foo/spec.md")
    techlead = _payload("docs/CODEX_PUSH_SMOKE.spec.md")
    prompt = build_design_chief_prompt(plan, techlead, {}, {"autofix": {}})
    assert "openspec/changes/foo/spec.md" in prompt
    assert "docs/CODEX_PUSH_SMOKE.spec.md" in prompt
    assert "NEVER cite" in prompt
