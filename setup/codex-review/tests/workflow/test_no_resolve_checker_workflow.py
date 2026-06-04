from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"


def test_no_separate_resolve_checker_workflow():
    names = [p.name for p in WORKFLOWS.glob("*")]
    assert "resolve-checker.yml" not in names
    assert "resolve-checker.yaml" not in names


def test_resolve_logic_is_stage_integrated_in_reusable_core():
    # Resolve/review logic is no longer a separate resolve-checker or split
    # review workflow; it is integrated into the single reusable core, gated by
    # the `stage` input rather than dedicated per-stage workflow files.
    yml_names = {p.name for p in WORKFLOWS.glob("*.yml")}
    for legacy in (
        "resolve-checker.yml",
        "codex-review.yml",
        "codex-design.yml",
        "codex-fix.yml",
        "codex-issue.yml",
    ):
        assert legacy not in yml_names, legacy
    reusable = (WORKFLOWS / "codex-loop-reusable.yml").read_text(encoding="utf-8")
    assert "stage:" in reusable
    assert "review|design|fix|issue" in reusable
    assert "Fix Stage Skeleton" not in reusable
    assert "Issue Fallback Artifact" in reusable
    for fix_job_name in (
        "Fix Dispatch Plan Tasks",
        "Fix Run Agents Matrix",
        "Fix Merge Validate",
    ):
        assert fix_job_name in reusable
    for review_job_name in (
        "Review Collect And Gate",
        "Review Findings By Axis",
        "Review Combine Findings",
        "Review Techlead Decision And Route",
    ):
        assert review_job_name in reusable
    for design_job_name in (
        "Design Prepare Clusters",
        "Design Analyze Clusters",
        "Design Draft Plan",
        "Design Chief Decision And Route",
    ):
        assert design_job_name in reusable
