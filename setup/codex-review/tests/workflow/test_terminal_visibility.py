"""Terminal and continuation visibility for the single-run reusable core.

The current core no longer has a ``finalize-stage`` job or terminal artifact
machinery. Terminal visibility is expressed through ``run-stage`` stage outputs,
``finalize`` reusable outputs, repository_dispatch payload JSON, and the final
``required-checks`` aggregator.
"""
from __future__ import annotations

from pathlib import Path
from typing import TypeAlias, cast

import yaml

YamlScalar: TypeAlias = str | int | bool | None
YamlValue: TypeAlias = YamlScalar | list["YamlValue"] | dict[str, "YamlValue"]
YamlMap: TypeAlias = dict[str, YamlValue]
YamlSteps: TypeAlias = list[YamlMap]

ROOT = Path(__file__).resolve().parents[4]
CORE = ROOT / ".github" / "workflows" / "codex-loop-reusable.yml"
ENTRY_INPUTS = ("pr_number", "head_sha", "base_ref", "iteration", "correlation_id", "requested_by")
CLASSIFY_NOOP_GUARDS = (
    '"${CLASSIFY_MEMORY_ONLY}" == "true"',
    '"${CLASSIFY_SHOULD_RUN_MODEL}" == "false"',
    '"${CLASSIFY_CODEX_MEMORY_MARKER}" == "true"',
    '"${CLASSIFY_ACTOR_GUARD}" == "true"',
)


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


def _required_str(mapping: YamlMap, key: str) -> str:
    value = mapping[key]
    assert isinstance(value, str), f"expected string at {key}"
    return value


def _optional_bool(mapping: YamlMap, key: str) -> bool | None:
    value = mapping.get(key)
    if value is None:
        return None
    assert isinstance(value, bool), f"expected bool at {key}"
    return value


def _doc() -> YamlMap:
    loaded = cast(object, yaml.safe_load(CORE.read_text(encoding="utf-8")))
    return _as_map(_normalize_yaml(loaded), "workflow")


def _jobs() -> dict[str, YamlMap]:
    jobs = _as_map(_doc()["jobs"], "jobs")
    return {job_name: _as_map(job, f"jobs.{job_name}") for job_name, job in jobs.items()}


def _job(name: str) -> YamlMap:
    return _jobs()[name]


def _steps(job_name: str) -> YamlSteps:
    return _as_steps(_job(job_name)["steps"], f"jobs.{job_name}.steps")


def _step_by_id(job_name: str, step_id: str) -> YamlMap:
    for step in _steps(job_name):
        if step.get("id") == step_id:
            return step
    raise AssertionError(f"step id not found: {job_name}.{step_id}")


def test_no_legacy_finalize_stage_or_terminal_artifact_machinery_remains():
    jobs = _jobs()
    assert "finalize-stage" not in jobs
    assert "finalize" in jobs
    combined = str(jobs)
    for token in (
        "terminal_state",
        "terminal_conclusion",
        "effective_outputs",
        "codex-loop-terminal",
        "terminal-summary.md",
        "selected_result",
    ):
        assert token not in combined


def test_stage_outputs_step_surfaces_single_run_terminal_reason():
    stage_outputs = _step_by_id("run-stage", "stage_outputs")
    env = _as_map(stage_outputs["env"], "stage_outputs.env")
    run = _required_str(stage_outputs, "run")

    assert env["REVIEW_NEXT_STAGE"] == "${{ steps.review_complete.outputs.next_stage }}"
    assert env["REVIEW_LGTM"] == "${{ steps.review_complete.outputs.lgtm }}"
    assert env["DESIGN_NEXT_STAGE"] == "${{ steps.design_complete.outputs.next_stage }}"
    assert env["FIX_NEXT_STAGE"] == "${{ steps.fix_complete.outputs.next_stage }}"
    assert env["PUSH_REDISPATCH"] == "${{ steps.fix_complete.outputs.should_redispatch }}"
    assert env["PUSH_UPDATED_HEAD_SHA"] == "${{ steps.fix_complete.outputs.updated_head_sha }}"

    for output_name in ("lgtm", "should_redispatch", "terminal_reason", "updated_head_sha"):
        assert f"echo \"{output_name}=${{{output_name}}}\"" in run
    assert 'terminal_reason="push_failed"' in run
    assert 'terminal_reason="model_output_invalid"' in run
    assert 'terminal_reason="${REVIEW_TERMINAL_REASON:-model_output_invalid}"' in run
    assert 'lgtm="true"' in run


def test_finalize_resolves_memory_only_noop_without_redispatch():
    finalize = _job("finalize")
    assert finalize["needs"] == ["validate", "setup-state", "classify", "run-stage"]
    resolve = _step_by_id("finalize", "resolve")
    env = _as_map(resolve["env"], "resolve.env")
    run = _required_str(resolve, "run")

    assert env["CLASSIFY_MEMORY_ONLY"] == "${{ needs.classify.outputs.memory_only }}"
    assert env["CLASSIFY_SHOULD_RUN_MODEL"] == "${{ needs.classify.outputs.should_run_model }}"
    assert env["CLASSIFY_CODEX_MEMORY_MARKER"] == "${{ needs.classify.outputs.codex_memory_marker }}"
    assert env["CLASSIFY_ACTOR_GUARD"] == "${{ needs.classify.outputs.actor_guard }}"
    for guard in CLASSIFY_NOOP_GUARDS:
        assert guard in run
    assert 'terminal_reason="memory_only_noop"' in run
    assert 'should_redispatch="false"' in run
    assert 'updated_head_sha=""' in run
    assert 'dispatch_candidate="false"' in run


def test_run_stage_records_non_pushed_terminal_memory_best_effort():
    terminal_memory = _step_by_id("run-stage", "terminal_memory")
    env = _as_map(terminal_memory["env"], "terminal_memory.env")
    run = _required_str(terminal_memory, "run")

    assert terminal_memory["if"] == "${{ always() && steps.stage_outputs.outputs.should_redispatch != 'true' && (steps.stage_outputs.outputs.lgtm == 'true' || steps.stage_outputs.outputs.terminal_reason != '') }}"
    assert _optional_bool(terminal_memory, "continue-on-error") is True
    assert env["COMMON_ARTIFACT_DIR"] == "codex-review-artifacts/common"
    assert env["REPO_PATH"] == "pr-head"
    assert env["TERMINAL_SHOULD_REDISPATCH"] == "${{ steps.stage_outputs.outputs.should_redispatch }}"
    assert "${CODEX_LOOP_PAT:-}" in run
    assert 'echo "::add-mask::${loop_pat}"' in run
    assert '"${TERMINAL_SHOULD_REDISPATCH}" == "true"' in run
    assert 'terminal memory skipped: pushed continuation' in run
    assert 'codex-review reentry record-reentry' in run
    assert '--in codex-review-artifacts/terminal-memory/terminal-output.json' in run
    assert '--loop-state "${LOOP_STATE_PATH}"' in run
    assert '--pr-context "${COMMON_ARTIFACT_DIR}/pr-context.json"' in run
    assert '--repo-path "${REPO_PATH}"' in run
    assert '--token "${loop_pat}"' in run
    assert 'terminal memory record-reentry failed nonfatally' in run


def test_finalize_repository_dispatch_payload_carries_only_next_iteration_inputs():
    resolve_run = _required_str(_step_by_id("finalize", "resolve"), "run")
    assert '--arg event_type "codex-loop"' in resolve_run
    assert "repository_dispatch client_payload carries only the next iteration" in resolve_run
    for key in ENTRY_INPUTS:
        assert f"--arg {key} " in resolve_run
        assert f"{key}:" in resolve_run
    assert "$stage" not in resolve_run
    assert "state_run_id" not in resolve_run
    assert "state_artifact_name" not in resolve_run


def test_finalize_dispatch_step_and_final_outputs_do_not_mask_dispatch_failures():
    dispatch = _step_by_id("finalize", "dispatch_continuation")
    final_outputs = _step_by_id("finalize", "final_outputs")
    run = _required_str(final_outputs, "run")
    dispatch_env = _as_map(final_outputs["env"], "final_outputs.env")

    assert dispatch["if"] == "${{ steps.resolve.outputs.dispatch_candidate == 'true' }}"
    assert 'gh api --method POST "repos/${REPOSITORY}/dispatches" --input .codex-loop-repository-dispatch.json' in _required_str(dispatch, "run")
    assert final_outputs["if"] == "${{ always() }}"
    assert dispatch_env["DISPATCH_CANDIDATE"] == "${{ steps.resolve.outputs.dispatch_candidate }}"
    assert dispatch_env["DISPATCHED"] == "${{ steps.dispatch_continuation.outputs.dispatched }}"
    assert 'terminal_reason="dispatch-not-emitted"' in run
    assert 'should_redispatch="false"' in run


def test_required_checks_aggregates_failures_and_allows_model_skip():
    required = _job("required-checks")
    assert required["needs"] == ["validate", "setup-state", "classify", "run-stage", "finalize"]
    assert required["if"] == "${{ !cancelled() }}"
    run = _required_str(_steps("required-checks")[0], "run")
    assert '.result == "failure" or .result == "cancelled"' in run
    assert 'exit 1' in run
    assert 'skipped' not in run.split('exit 1')[0]
    assert "all required jobs succeeded or skipped" in run


def test_finalize_terminal_paths_do_not_introduce_secret_or_success_masking():
    for step in _steps("finalize"):
        env_text = str(step.get("env", {}))
        assert "secrets." not in env_text
        assert "APP_PRIVATE_KEY" not in env_text
        assert "APP_TOKEN" not in env_text
        assert "relay_token" not in env_text
        assert "github.token" not in env_text
        assert _optional_bool(step, "continue-on-error") is not True
        if "run" in step:
            assert "exit 0" not in _required_str(step, "run")
