import pytest
from codex_review.stages.design.coordinate import validate_design_plan
from codex_review.stages.design.cluster import validate_design_clusters

CFG={"design":{},"autofix":{"dangerous_keywords":[]}}
CTX={"findings":[{"finding_id":"f1"}]}


def write_repo_file(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.rs").write_text("pub fn demo() {}\n", encoding="utf-8")


def plan_with_evidence(**extra):
    payload = {
        "edit_sequence":[{"task_id":"t1","allowed_files":["src/a.rs"]}],
        "tests":["pytest"],
        "inspection_evidence":[
            {
                "path":"src/a.rs",
                "purpose":"Inspect the target file for the planned edit",
                "observation":"The file is the bounded implementation surface.",
            }
        ],
    }
    payload.update(extra)
    return payload


def test_design_plan_requires_tests_for_findings(tmp_path):
    write_repo_file(tmp_path)
    with pytest.raises(Exception):
        validate_design_plan(plan_with_evidence(tests=[]), CTX, CFG, repo_path=tmp_path)


def test_design_plan_adds_hash_and_validates_inspection_evidence(tmp_path):
    write_repo_file(tmp_path)
    out=validate_design_plan(plan_with_evidence(), CTX, CFG, repo_path=tmp_path)
    assert out["plan_hash"]
    assert out["inspection_evidence"][0]["path"] == "src/a.rs"


def test_design_plan_requires_inspection_evidence(tmp_path):
    with pytest.raises(Exception, match="inspection_evidence"):
        validate_design_plan({"edit_sequence":[{"task_id":"t1"}],"tests":["pytest"]}, CTX, CFG, repo_path=tmp_path)


def test_design_plan_carries_openspec_acceptance_contract(tmp_path):
    write_repo_file(tmp_path)
    ctx={**CTX,"openspec_backed":True,"openspec_context":{"source_summary":["openspec/changes/demo/tasks.md"]}}
    out=validate_design_plan(
        plan_with_evidence(
            edit_sequence=[{"task_id":"t1","allowed_files":["src/a.rs"],"acceptance_criteria":["spec passes"]}],
        ),
        ctx,
        CFG,
        repo_path=tmp_path,
    )
    assert out["openspec_backed"] is True
    assert out["acceptance_criteria"] == ["spec passes"]
    assert out["openspec_sources"] == ["openspec/changes/demo/tasks.md"]


def test_design_plan_rejects_open_questions_field(tmp_path):
    write_repo_file(tmp_path)
    with pytest.raises(Exception, match="does not accept open_questions"):
        validate_design_plan(plan_with_evidence(open_questions=["What about API?"]), CTX, CFG, repo_path=tmp_path)


def test_clusters_must_cover_inventory():
    with pytest.raises(Exception):
        validate_design_clusters({"clusters":[]}, {"items":[{"finding_id":"f1"}]})
