import pytest
from codex_review.stages.review.validate import validate_axis_findings
from codex_review.stages.review.combine import combine_axis_findings

CFG={"review":{"axes":["correctness"],"max_findings_per_axis":3,"require_changed_right_line":True}}


def finding(fid="f1", line=10):
    return {"finding_id":fid,"severity":"medium","file":"src/a.py","line":line,"root_cause_key":"rc1","title":"Bug","summary":"Bug","recommendation":"Fix"}


def payload_with_evidence(findings):
    return {
        "axis": "correctness",
        "findings": findings,
        "inspection_evidence": [
            {
                "path": "src/a.py",
                "purpose": "Inspect implementation touched by the finding",
                "observation": "The changed line is relevant to the reported bug.",
            }
        ],
    }


def payload_with_custom_evidence(findings, path, axis="correctness"):
    payload = payload_with_evidence(findings)
    payload["axis"] = axis
    payload["inspection_evidence"][0]["path"] = str(path)
    return payload


def test_axis_findings_validate_location_shape_and_inspection_evidence(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.py").write_text("print('ok')\n", encoding="utf-8")
    out=validate_axis_findings("correctness", payload_with_evidence([finding()]), {}, {"src/a.py":[10]}, CFG, repo_path=tmp_path)
    assert out["finding_count"] == 1
    assert out["inspection_evidence"][0]["path"] == "src/a.py"


def test_axis_findings_normalize_absolute_inspection_path_inside_repo(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    evidence_file = src / "a.py"
    evidence_file.write_text("print('ok')\n", encoding="utf-8")

    out = validate_axis_findings(
        "correctness",
        payload_with_custom_evidence([], evidence_file),
        {},
        {},
        CFG,
        repo_path=tmp_path,
    )

    assert out["inspection_evidence"][0]["path"] == "src/a.py"


def test_axis_findings_accept_domain_display_axis_alias(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.py").write_text("print('ok')\n", encoding="utf-8")

    out = validate_axis_findings(
        "domain",
        payload_with_custom_evidence(
            [],
            "src/a.py",
            axis="project-specific correctness and product requirements",
        ),
        {},
        {},
        {"review":{"axes":["domain"],"max_findings_per_axis":3,"require_changed_right_line":True}},
        repo_path=tmp_path,
    )

    assert out["axis"] == "domain"


def test_axis_findings_accept_project_specific_correctness_axis_alias(tmp_path):
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.py").write_text("print('ok')\n", encoding="utf-8")

    out = validate_axis_findings(
        "domain",
        payload_with_custom_evidence(
            [],
            "src/a.py",
            axis="project-specific-correctness",
        ),
        {},
        {},
        {"review":{"axes":["domain"],"max_findings_per_axis":3,"require_changed_right_line":True}},
        repo_path=tmp_path,
    )

    assert out["axis"] == "domain"


def test_axis_findings_reject_unchanged_line():
    with pytest.raises(Exception):
        validate_axis_findings("correctness", payload_with_evidence([finding(line=11)]), {}, {"src/a.py":[10]}, CFG)


def test_axis_findings_require_inspection_evidence_even_when_no_findings(tmp_path):
    with pytest.raises(Exception, match="inspection_evidence"):
        validate_axis_findings("correctness", {"axis":"correctness","findings":[]}, {}, {}, CFG, repo_path=tmp_path)


def test_axis_findings_reject_missing_inspection_path(tmp_path):
    with pytest.raises(Exception, match="inspection_evidence path does not exist"):
        validate_axis_findings("correctness", payload_with_evidence([]), {}, {}, CFG, repo_path=tmp_path)


def test_combine_rejects_duplicate_ids():
    a={"axis":"correctness","findings":[finding("f1")]}
    b={"axis":"correctness","findings":[finding("f1")]}
    with pytest.raises(Exception):
        combine_axis_findings([a,b])


def test_axis_findings_allow_more_than_old_cap(tmp_path):
    # The per-axis findings cap (was 3 in CFG) was removed: many findings on one
    # axis must all validate instead of failing the review.
    src = tmp_path / "src"
    src.mkdir()
    (src / "a.py").write_text("\n".join(f"line{i}" for i in range(60)), encoding="utf-8")
    findings = [finding(fid=f"f{i}", line=i) for i in range(1, 31)]
    changed = {"src/a.py": list(range(1, 31))}
    out = validate_axis_findings("correctness", payload_with_evidence(findings), {}, changed, CFG, repo_path=tmp_path)
    assert out["finding_count"] == 30
