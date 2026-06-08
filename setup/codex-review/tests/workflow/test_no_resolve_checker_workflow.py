from pathlib import Path
from typing import TypeAlias, cast

import yaml

YamlScalar: TypeAlias = str | int | bool | None
YamlValue: TypeAlias = YamlScalar | list["YamlValue"] | dict[str, "YamlValue"]
YamlMap: TypeAlias = dict[str, YamlValue]
YamlStep: TypeAlias = YamlMap
YamlSteps: TypeAlias = list[YamlStep]

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"
CORE = WORKFLOWS / "codex-loop-reusable.yml"
TRUSTED_CODEX_USES = {
    "./trusted-core/.github/actions/codex-memory-classify",
    "./trusted-core/.github/actions/codex-context",
    "./trusted-core/.github/actions/codex-review-phase",
    "./trusted-core/.github/actions/codex-design-phase",
    "./trusted-core/.github/actions/codex-fix-phase",
}


def _yaml_key(key: object) -> str:
    if key is True:
        return "on"
    assert isinstance(key, str), f"expected YAML string key, got {key!r}"
    return key


def _normalize_yaml(value: object) -> YamlValue:
    if isinstance(value, str) or isinstance(value, bool) or isinstance(value, int) or value is None:
        return value
    if isinstance(value, list):
        return [_normalize_yaml(item) for item in cast(list[object], value)]
    if isinstance(value, dict):
        return {
            _yaml_key(key): _normalize_yaml(item)
            for key, item in cast(dict[object, object], value).items()
        }
    raise AssertionError(f"unsupported YAML value: {value!r}")


def _as_map(value: YamlValue, label: str) -> YamlMap:
    assert isinstance(value, dict), f"expected mapping at {label}"
    return value


def _as_steps(value: YamlValue, label: str) -> YamlSteps:
    assert isinstance(value, list), f"expected step list at {label}"
    steps: YamlSteps = []
    for item in value:
        assert isinstance(item, dict), f"expected step mapping at {label}"
        steps.append(item)
    return steps


def _load_yaml(path: Path) -> YamlMap:
    loaded = cast(object, yaml.safe_load(path.read_text(encoding="utf-8")))
    return _as_map(_normalize_yaml(loaded), "workflow")


def _workflow_call(doc: YamlMap) -> YamlMap:
    return _as_map(_as_map(doc["on"], "on")["workflow_call"], "workflow_call")


def _steps(job: YamlMap) -> YamlSteps:
    return _as_steps(job.get("steps") or [], "steps")


def test_no_separate_resolve_checker_workflow():
    names = [p.name for p in WORKFLOWS.glob("*")]
    assert "resolve-checker.yml" not in names
    assert "resolve-checker.yaml" not in names


def test_resolve_and_model_logic_are_integrated_in_single_reusable_core():
    yml_names = {p.name for p in WORKFLOWS.glob("*.yml")}
    for legacy in (
        "resolve-checker.yml",
        "codex-review.yml",
        "codex-design.yml",
        "codex-fix.yml",
        "codex-issue.yml",
    ):
        assert legacy not in yml_names, legacy

    doc = _load_yaml(CORE)
    jobs = _as_map(doc["jobs"], "jobs")
    assert list(jobs) == ["validate", "setup-state", "classify", "run-stage", "finalize", "required-checks"]
    call_inputs = _as_map(_workflow_call(doc)["inputs"], "workflow_call.inputs")
    assert "stage" not in call_inputs
    assert "state_run_id" not in call_inputs
    assert "state_artifact_name" not in call_inputs

    run_stage_uses: set[str] = set()
    for step in _steps(_as_map(jobs["run-stage"], "jobs.run-stage")):
        uses = step.get("uses")
        if isinstance(uses, str) and uses.startswith("./trusted-core/.github/actions/codex-"):
            run_stage_uses.add(uses)

    classify_uses: set[str] = set()
    for step in _steps(_as_map(jobs["classify"], "jobs.classify")):
        uses = step.get("uses")
        if isinstance(uses, str) and uses.startswith("./trusted-core/.github/actions/codex-"):
            classify_uses.add(uses)

    assert run_stage_uses | classify_uses == TRUSTED_CODEX_USES

    reusable = CORE.read_text(encoding="utf-8")
    assert "review|design|fix|issue" not in reusable
    assert "Issue Fallback Artifact" not in reusable
    assert "Fix Stage Skeleton" not in reusable
    for legacy_job_name in (
        "Review Collect And Gate",
        "Review Findings By Axis",
        "Review Combine Findings",
        "Review Techlead Decision And Route",
        "Design Prepare Clusters",
        "Design Analyze Clusters",
        "Design Draft Plan",
        "Design Chief Decision And Route",
        "Fix Dispatch Plan Tasks",
        "Fix Run Agents Matrix",
        "Fix Merge Validate",
    ):
        assert legacy_job_name not in reusable
