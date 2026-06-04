import pytest
from codex_review.stages.techlead.validate import validate_techlead_decision
from codex_review.stages.techlead.classify import build_review_publication

COMBINED={"findings":[{"finding_id":"f1","file":"src/a.py","line":1,"root_cause_key":"r","title":"T","summary":"S"}]}
CFG={"autofix":{"dangerous_keywords":["secret","auth","nonce"]}}


def evidence_payload(decisions):
    return {
        "decisions": decisions,
        "inspection_evidence": [
            {
                "path": "src/a.py",
                "purpose": "Check the file referenced by the combined finding",
                "observation": "The finding is implementable from the current PR context.",
            }
        ],
    }


def write_repo_file(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.py").write_text("print('ok')\n", encoding="utf-8")


def test_techlead_requires_finding_coverage(tmp_path):
    write_repo_file(tmp_path)
    with pytest.raises(Exception):
        validate_techlead_decision(evidence_payload([]), COMBINED, CFG, repo_path=tmp_path)


def test_techlead_validates_action_publication_and_inspection_evidence(tmp_path):
    write_repo_file(tmp_path)
    decision=validate_techlead_decision(evidence_payload([{"finding_id":"f1","action":"publish_only"}]), COMBINED, CFG, repo_path=tmp_path)
    pub=build_review_publication(decision, COMBINED, CFG)
    assert pub["inline_comments"][0]["finding_id"] == "f1"
    assert decision["inspection_evidence"][0]["path"] == "src/a.py"


def test_techlead_requires_inspection_evidence(tmp_path):
    with pytest.raises(Exception, match="inspection_evidence"):
        validate_techlead_decision({"decisions":[{"finding_id":"f1","action":"publish_only"}]}, COMBINED, CFG, repo_path=tmp_path)


def test_semantic_risk_does_not_block_fix_routing(tmp_path):
    write_repo_file(tmp_path)
    decision=validate_techlead_decision(evidence_payload([{"finding_id":"f1","action":"publish_and_fix_now","risk":"secret/auth/nonce handling"}]), COMBINED, CFG, repo_path=tmp_path)
    assert decision["status"] == "needs_design"
    assert decision["decisions"][0]["semantic_risk_hints"] == ["auth", "nonce", "secret"]


def test_generic_needs_human_is_normalized_to_design_for_openspec_loop(tmp_path):
    write_repo_file(tmp_path)
    decision=validate_techlead_decision(evidence_payload([{"finding_id":"f1","action":"needs_human","reason":"uncertain, but implementable"}]), COMBINED, CFG, repo_path=tmp_path)
    assert decision["status"] == "needs_design"
    assert decision["decisions"][0]["action"] == "needs_design"
    assert decision["decisions"][0]["normalized_from"] == "needs_human"


def test_explicit_non_executable_blocker_can_still_stop_for_human(tmp_path):
    write_repo_file(tmp_path)
    decision=validate_techlead_decision(evidence_payload([{"finding_id":"f1","action":"needs_human","blocker_type":"secret_required"}]), COMBINED, CFG, repo_path=tmp_path)
    assert decision["status"] == "needs_human"
    assert decision["decisions"][0]["action"] == "needs_human"
