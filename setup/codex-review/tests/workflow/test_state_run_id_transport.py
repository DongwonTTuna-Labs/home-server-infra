"""Workflow contract tests for the current single-run continuation payload."""
from __future__ import annotations

from pathlib import Path
from typing import TypeAlias, cast

import yaml

YamlScalar: TypeAlias = str | int | bool | None
YamlValue: TypeAlias = YamlScalar | list["YamlValue"] | dict[str, "YamlValue"]
YamlMap: TypeAlias = dict[str, YamlValue]
YamlSteps: TypeAlias = list[YamlMap]

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"
ENTRY_INPUTS = ("pr_number", "head_sha", "base_ref", "iteration", "correlation_id", "requested_by")
DELETED_POINTERS = ("state_run_id", "state_artifact_name")


def _text(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


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


def _load(name: str) -> YamlMap:
    loaded = cast(object, yaml.safe_load(_text(name)))
    return _as_map(_normalize_yaml(loaded), "workflow")


def _workflow_call_inputs(name: str) -> YamlMap:
    doc = _load(name)
    call = _as_map(_as_map(doc["on"], "on")["workflow_call"], "workflow_call")
    return _as_map(call["inputs"], "workflow_call.inputs")


def test_dispatch_payload_validates_and_forwards_only_entry_inputs():
    doc = _load("codex-loop-dispatch.yml")
    jobs = _as_map(doc["jobs"], "jobs")
    validate = _as_map(jobs["validate-payload"], "jobs.validate-payload")
    call = _as_map(jobs["codex-loop"], "jobs.codex-loop")
    outputs = _as_map(validate["outputs"], "validate.outputs")
    steps = _as_steps(validate["steps"], "validate.steps")
    validate_step = _as_map(steps[0], "validate.steps[0]")
    env = _as_map(validate_step["env"], "validate.env")
    run = validate_step["run"]
    assert isinstance(run, str)
    with_map = _as_map(call["with"], "codex-loop.with")
    dispatch_text = _text("codex-loop-dispatch.yml")

    assert tuple(outputs) == ENTRY_INPUTS
    assert tuple(with_map) == ENTRY_INPUTS
    for field in ENTRY_INPUTS:
        upper = field.upper()
        assert f"PAYLOAD_{upper}: ${{{{ github.event.client_payload.{field} }}}}" in dispatch_text
        assert f"require_non_empty {field} " in run
        assert field in outputs
        assert field in with_map

    assert "PAYLOAD_STAGE" not in env
    assert "github.event.client_payload.stage" not in dispatch_text
    assert "review|design|fix|issue" not in run
    for field in DELETED_POINTERS:
        assert field not in outputs
        assert field not in with_map
        assert f"invalid-{field}" not in run
        assert f"github.event.client_payload.{field}" not in dispatch_text


def test_reusable_workflow_uses_fresh_run_local_state_not_prior_state_pointer():
    inputs = _workflow_call_inputs("codex-loop-reusable.yml")
    for field in DELETED_POINTERS:
        assert field not in inputs
    assert "stage" not in inputs
    assert tuple(inputs) == ENTRY_INPUTS

    text = _text("codex-loop-reusable.yml")
    assert "run-id: ${{ inputs.state_run_id }}" not in text
    assert "name: ${{ inputs.state_artifact_name }}" not in text
    assert "--loop-state \"codex-review-artifacts/prior-state/" not in text
    assert "Download prior loop state artifact" not in text
    assert "codex-review loop read-state --out \"${RUNNER_TEMP}/codex-loop-state.json\"" in text
    assert "name: codex-loop-state-${{ inputs.correlation_id }}-${{ inputs.iteration }}.json" in text
    assert "${{ runner.temp }}/codex-loop-state.json" in text


def test_manual_workflow_carries_only_entry_inputs_to_reusable_core():
    doc = _load("codex-loop-manual.yml")
    workflow_dispatch = _as_map(_as_map(doc["on"], "on")["workflow_dispatch"], "workflow_dispatch")
    inputs = _as_map(workflow_dispatch["inputs"], "workflow_dispatch.inputs")
    jobs = _as_map(doc["jobs"], "jobs")
    manual = _as_map(jobs["manual-debug-adapter"], "jobs.manual-debug-adapter")
    with_map = _as_map(manual["with"], "manual.with")

    assert tuple(inputs) == ENTRY_INPUTS
    assert tuple(with_map) == ENTRY_INPUTS
    for field in ENTRY_INPUTS:
        assert field in inputs
        assert with_map[field] == f"${{{{ inputs.{field} }}}}"
    assert "stage" not in inputs
    for field in DELETED_POINTERS:
        assert field not in inputs
        assert field not in with_map


def test_active_workflows_do_not_use_head_sha_artifact_discovery_or_old_pointers():
    combined = "\n".join(_text(name) for name in ("codex-loop-dispatch.yml", "codex-loop-reusable.yml", "codex-loop-manual.yml"))

    for marker in ("gh run list", "--headSha", "headSha"):
        assert marker not in combined
    assert "inputs.stage" not in _text("codex-loop-reusable.yml")
    assert "github.event.client_payload.stage" not in _text("codex-loop-dispatch.yml")
    assert "state_run_id" not in _text("codex-loop-dispatch.yml")
    assert "state_artifact_name" not in _text("codex-loop-dispatch.yml")
    assert "state_run_id" not in _text("codex-loop-manual.yml")
    assert "state_artifact_name" not in _text("codex-loop-manual.yml")
