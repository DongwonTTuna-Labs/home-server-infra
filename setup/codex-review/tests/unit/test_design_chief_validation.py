import pytest
from codex_review.stages.design_chief.validate import validate_chief_decision
from codex_review.stages.design_chief.route import route_after_design_chief

CFG={"autofix":{"allowed_prefixes":["src/"],"max_tasks":3}}
PLAN={"edit_sequence":[{"task_id":"t1","files":["src/a.py"]}],"tests":["pytest"]}


def write_repo_file(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.py").write_text("print('ok')\n", encoding="utf-8")


def decision_with_evidence(**extra):
    decision = {
        "inspection_evidence": [
            {
                "path": "src/a.py",
                "purpose": "Inspect the bounded fix surface before approving",
                "observation": "The planned fix remains within the reviewed file surface.",
            }
        ]
    }
    decision.update(extra)
    return decision


def test_approval_requires_fix_policy_and_inspection_evidence(tmp_path):
    write_repo_file(tmp_path)
    out=validate_chief_decision(decision_with_evidence(status="approved_for_fix",fix_policy={"allowed_files":["src/a.py"],"max_tasks":1}), PLAN, CFG, repo_path=tmp_path)
    assert out["status"] == "approved_for_fix"
    assert out["inspection_evidence"][0]["path"] == "src/a.py"
    assert route_after_design_chief(out)["route"] == "run_fix_dispatch"


def test_chief_decision_requires_inspection_evidence(tmp_path):
    with pytest.raises(Exception, match="inspection_evidence"):
        validate_chief_decision({"status":"approved_for_fix","fix_policy":{"allowed_files":["src/a.py"],"max_tasks":1}}, PLAN, CFG, repo_path=tmp_path)


def test_requires_human_review_blocks_non_openspec_approval(tmp_path):
    write_repo_file(tmp_path)
    plan={**PLAN,"requires_human_review":True}
    with pytest.raises(Exception):
        validate_chief_decision(decision_with_evidence(status="approved_for_fix",fix_policy={"allowed_files":["src/a.py"],"max_tasks":1}), plan, CFG, repo_path=tmp_path)


def test_openspec_backed_executable_plan_is_promoted_to_fix_route(tmp_path):
    write_repo_file(tmp_path)
    plan={**PLAN,"openspec_backed":True,"acceptance_criteria":["OpenSpec task is satisfied"],"execution_blockers":[]}
    out=validate_chief_decision(decision_with_evidence(status="needs_human",reason="Model asked for confirmation"), plan, CFG, repo_path=tmp_path)
    assert out["status"] == "approved_for_fix"
    assert out["normalized_from"] == "needs_human"
    assert out["fix_policy"]["allowed_prefixes"] == ["src/"]
    assert route_after_design_chief(out)["route"] == "run_fix_dispatch"



def test_openspec_backed_generic_requires_human_review_flag_is_not_a_stop(tmp_path):
    write_repo_file(tmp_path)
    plan={**PLAN,"openspec_backed":True,"requires_human_review":True,"acceptance_criteria":["OpenSpec task is satisfied"],"execution_blockers":[]}
    out=validate_chief_decision(decision_with_evidence(status="needs_human",reason="generic review requested"), plan, CFG, repo_path=tmp_path)
    assert out["status"] == "approved_for_fix"


def test_openspec_backed_no_fix_or_rejected_plan_is_promoted_when_executable(tmp_path):
    write_repo_file(tmp_path)
    plan={**PLAN,"openspec_backed":True,"acceptance_criteria":["OpenSpec task is satisfied"],"execution_blockers":[]}
    for status in ["no_fix_needed", "rejected_plan"]:
        out=validate_chief_decision(decision_with_evidence(status=status,reason="too conservative"), plan, CFG, repo_path=tmp_path)
        assert out["status"] == "approved_for_fix"
        assert out["normalized_from"] == status


def test_openspec_backed_plan_with_execution_blocker_can_still_stop_for_human(tmp_path):
    write_repo_file(tmp_path)
    plan={**PLAN,"openspec_backed":True,"execution_blockers":["missing secret-bearing fixture"]}
    out=validate_chief_decision(decision_with_evidence(status="needs_human",reason="Secret fixture is unavailable"), plan, CFG, repo_path=tmp_path)
    assert out["status"] == "needs_human"
