"""Contract for the single-run sequential Codex loop core.

Private-repo infra: no OIDC, no GitHub App, no trust/security gates, no loop cap.
The core remains a four-job sequential pipeline — validate -> setup-state ->
run-stage -> finalize — with control-flow jobs classify and required-checks around
it. A SINGLE run executes review -> design -> fix -> push SEQUENTIALLY inside the
one run-stage job (skipping stages that aren't needed); repository_dispatch is used
ONLY to start the NEXT loop iteration after a remote-verified push. It runs live
via a static relay key and a permanent PAT, looping until LGTM.
"""
from pathlib import Path
from typing import TypeAlias, cast

import yaml

YamlScalar: TypeAlias = str | int | bool | None
YamlValue: TypeAlias = YamlScalar | list["YamlValue"] | dict[str, "YamlValue"]
YamlMap: TypeAlias = dict[str, YamlValue]
YamlStep: TypeAlias = YamlMap
YamlSteps: TypeAlias = list[YamlStep]

ROOT = Path(__file__).resolve().parents[4]
CORE = ROOT / ".github" / "workflows" / "codex-loop-reusable.yml"
CONTEXT_ACTION = ROOT / ".github" / "actions" / "codex-context" / "action.yml"
MEMORY_ACTION = ROOT / ".github" / "actions" / "codex-memory-classify" / "action.yml"
REVIEW_ACTION = ROOT / ".github" / "actions" / "codex-review-phase" / "action.yml"
DESIGN_ACTION = ROOT / ".github" / "actions" / "codex-design-phase" / "action.yml"
FIX_ACTION = ROOT / ".github" / "actions" / "codex-fix-phase" / "action.yml"
RELAY_ENDPOINT = "https://relay-ai.dongwontuna.net/v1/responses"
MODEL_ACTION = "openai/codex-action@v1"
MEMORY_CONTEXT = "codex-review-artifacts/common/memory-context.md"
TRUSTED_SOURCE_CHECKOUT_WITH = {
    "repository": "${{ steps.trusted_source.outputs.repository }}",
    "ref": "${{ steps.trusted_source.outputs.sha }}",
    "path": "trusted-core",
    "token": "${{ steps.read_pat.outputs.token }}",
    "persist-credentials": False,
}
REVIEW_PROMPT_BUILDERS_WITH_MEMORY = (
    ("prepare_stage", "codex-review review build-review-prompt"),
    ("review_techlead_prepare", "codex-review techlead build-techlead-prompt"),
)
DESIGN_PROMPT_BUILDERS_WITH_MEMORY = (
    ("design_inventory_prepare", "codex-review design build-inventory-prompt"),
    ("design_clusters_prepare", "codex-review design build-clusters-prompt"),
    ("design_plan_prepare", "codex-review design build-plan-prompt"),
    ("design_chief_prepare", "codex-review design_chief build-chief-prompt"),
)
FIX_PROMPT_BUILDERS_WITH_MEMORY = (
    ("fix_dispatch_prepare", "codex-review fix_dispatch prepare-agents"),
    ("fix_merge_prepare", "codex-review fix_merge prepare-merge-model"),
    ("fix_semantic_prepare", "codex-review fix_merge build-semantic-safety-prompt"),
)
PROMPT_BUILDERS_WITH_MEMORY = (
    REVIEW_PROMPT_BUILDERS_WITH_MEMORY
    + DESIGN_PROMPT_BUILDERS_WITH_MEMORY
    + FIX_PROMPT_BUILDERS_WITH_MEMORY
)


def _text() -> str:
    return CORE.read_text(encoding="utf-8")


def _yaml_key(key: object) -> str:
    if key is True:
        return "on"
    assert isinstance(key, str), f"expected YAML string key, got {key!r}"
    return key


def _normalize_yaml(value: object) -> YamlValue:
    if (
        isinstance(value, str)
        or isinstance(value, bool)
        or isinstance(value, int)
        or value is None
    ):
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


def _required_map(mapping: YamlMap, key: str) -> YamlMap:
    return _as_map(mapping[key], key)


def _optional_map(mapping: YamlMap, key: str) -> YamlMap:
    value = mapping.get(key)
    if value is None:
        return {}
    return _as_map(value, key)


def _required_steps(mapping: YamlMap, key: str) -> YamlSteps:
    return _as_steps(mapping[key], key)


def _optional_steps(mapping: YamlMap, key: str) -> YamlSteps:
    value = mapping.get(key)
    if value is None:
        return []
    return _as_steps(value, key)


def _required_str(mapping: YamlMap, key: str) -> str:
    value = mapping[key]
    assert isinstance(value, str), f"expected string at {key}"
    return value


def _optional_str(mapping: YamlMap, key: str) -> str:
    value = mapping.get(key)
    if value is None:
        return ""
    assert isinstance(value, str), f"expected string at {key}"
    return value


def _required_str_list(mapping: YamlMap, key: str) -> list[str]:
    value = mapping[key]
    assert isinstance(value, list), f"expected string list at {key}"
    result: list[str] = []
    for item in value:
        assert isinstance(item, str), f"expected string item at {key}"
        result.append(item)
    return result


def _load_yaml_map(text: str) -> YamlMap:
    loaded = cast(object, yaml.safe_load(text))
    return _as_map(_normalize_yaml(loaded), "document")


def _doc() -> YamlMap:
    return _load_yaml_map(_text())


def _context_action_text() -> str:
    return CONTEXT_ACTION.read_text(encoding="utf-8")


def _memory_action_text() -> str:
    return MEMORY_ACTION.read_text(encoding="utf-8")


def _review_action_text() -> str:
    return REVIEW_ACTION.read_text(encoding="utf-8")


def _design_action_text() -> str:
    return DESIGN_ACTION.read_text(encoding="utf-8")


def _fix_action_text() -> str:
    return FIX_ACTION.read_text(encoding="utf-8")


def _context_action_doc() -> YamlMap:
    return _load_yaml_map(_context_action_text())


def _memory_action_doc() -> YamlMap:
    return _load_yaml_map(_memory_action_text())


def _review_action_doc() -> YamlMap:
    return _load_yaml_map(_review_action_text())


def _design_action_doc() -> YamlMap:
    return _load_yaml_map(_design_action_text())


def _fix_action_doc() -> YamlMap:
    return _load_yaml_map(_fix_action_text())


def _workflow_call() -> YamlMap:
    return _required_map(_required_map(_doc(), "on"), "workflow_call")


def _workflow_call_inputs() -> YamlMap:
    return _required_map(_workflow_call(), "inputs")


def _jobs() -> dict[str, YamlMap]:
    jobs = _required_map(_doc(), "jobs")
    result: dict[str, YamlMap] = {}
    for name, job in jobs.items():
        result[name] = _as_map(job, f"jobs.{name}")
    return result


def _job(name: str) -> YamlMap:
    return _jobs()[name]


def _context_action_run() -> str:
    runs = _required_map(_context_action_doc(), "runs")
    return _required_str(_step_by_id(_required_steps(runs, "steps"), "build"), "run")


def _memory_action_steps() -> YamlSteps:
    runs = _required_map(_memory_action_doc(), "runs")
    return _required_steps(runs, "steps")


def _review_action_steps() -> YamlSteps:
    runs = _required_map(_review_action_doc(), "runs")
    return _required_steps(runs, "steps")


def _design_action_steps() -> YamlSteps:
    runs = _required_map(_design_action_doc(), "runs")
    return _required_steps(runs, "steps")


def _fix_action_steps() -> YamlSteps:
    runs = _required_map(_fix_action_doc(), "runs")
    return _required_steps(runs, "steps")


def _memory_action_run(step_id: str) -> str:
    return _required_str(_step_by_id(_memory_action_steps(), step_id), "run")


def _review_action_run(step_id: str) -> str:
    return _required_str(_step_by_id(_review_action_steps(), step_id), "run")


def _design_action_run(step_id: str) -> str:
    return _required_str(_step_by_id(_design_action_steps(), step_id), "run")


def _fix_action_run(step_id: str) -> str:
    return _required_str(_step_by_id(_fix_action_steps(), step_id), "run")


def _run_stage_steps() -> YamlSteps:
    return _required_steps(_job("run-stage"), "steps")


def _step_by_id(steps: YamlSteps, step_id: str) -> YamlStep:
    for step in steps:
        if step.get("id") == step_id:
            return step
    raise AssertionError(f"step id not found: {step_id}")


def _step_index_by_id(steps: YamlSteps, step_id: str) -> int:
    for index, step in enumerate(steps):
        if step.get("id") == step_id:
            return index
    raise AssertionError(f"step id not found: {step_id}")


def _assert_trusted_source_resolver(steps: YamlSteps, checkout: YamlStep) -> None:
    resolver = _step_by_id(steps, "trusted_source")
    assert _required_str(resolver, "name") == "Resolve trusted workflow source"
    assert _optional_str(resolver, "shell") == "bash"
    assert _required_map(resolver, "env") == {"JOB_CONTEXT": "${{ toJson(job) }}"}
    run = _required_str(resolver, "run")
    for required in (
        '"workflow_repository"',
        '"workflow_sha"',
        "trusted workflow repository is missing or invalid",
        "trusted workflow sha is missing or invalid",
        "repository=${workflow_repository}",
        "sha=${workflow_sha}",
    ):
        assert required in run
    assert "${{ job.workflow_repository }}" not in run
    assert "${{ job.workflow_sha }}" not in run
    assert steps.index(resolver) < steps.index(checkout)


def _command_block(run: str, command: str) -> str:
    start = run.index(command)
    block: list[str] = []
    for line in run[start:].splitlines():
        stripped = line.strip()
        if not block:
            block.append(stripped)
            continue
        if stripped.startswith("--"):
            block.append(stripped)
            continue
        break
    return "\n".join(block)


def test_core_pipeline_and_control_jobs_in_order():
    jobs = _jobs()
    job_names = list(jobs.keys())
    assert job_names == ["validate", "setup-state", "classify", "run-stage", "finalize", "required-checks"], job_names
    assert _required_str_list(jobs["classify"], "needs") == ["validate", "setup-state"]
    assert _required_str_list(jobs["run-stage"], "needs") == ["validate", "setup-state", "classify"]
    assert _required_str_list(jobs["finalize"], "needs") == ["validate", "setup-state", "classify", "run-stage"]


def test_trusted_workflow_source_has_no_direct_job_property_expressions():
    text = _text()
    assert "${{ job.workflow_repository }}" not in text
    assert "${{ job.workflow_sha }}" not in text
    assert "${{ toJson(job) }}" in text


def test_no_matrix_or_fromjson_fanout_added():
    texts = (
        _text(), _context_action_text(), _memory_action_text(),
        _review_action_text(), _design_action_text(), _fix_action_text(),
    )
    for text in texts:
        for line in text.splitlines():
            stripped = line.strip()
            assert not stripped.startswith("strategy:"), line
            assert not stripped.startswith("matrix:"), line
        assert "fromJson" not in text


def test_common_memory_context_built_after_pr_context_before_prompt_builders():
    steps = _run_stage_steps()
    context_step = _step_by_id(steps, "codex_context")
    context_with = _required_map(context_step, "with")
    review_step = _step_by_id(steps, "review_complete")
    review_with = _required_map(review_step, "with")
    review_prepare = _review_action_run("prepare_stage")
    context_run = _context_action_run()
    pr_context_out = '--out "${ARTIFACT_DIR}/pr-context.json"'
    memory_command = "codex-review context memory"
    first_prompt_builder = "codex-review review build-review-prompt"

    assert _required_str(context_step, "uses") == "./trusted-core/.github/actions/codex-context"
    assert _required_str(context_with, "repo-path") == "pr-head"
    assert _required_str(context_with, "artifact-dir") == "codex-review-artifacts/common"
    assert context_run.index(pr_context_out) < context_run.index(memory_command)
    assert (
        _step_index_by_id(steps, "codex_context")
        < _step_index_by_id(steps, "review_complete")
    )
    assert _required_str(review_step, "uses") == "./trusted-core/.github/actions/codex-review-phase"
    assert _required_str(review_with, "memory-context") == MEMORY_CONTEXT
    assert first_prompt_builder in review_prepare

    memory_block = _command_block(context_run, memory_command)
    assert '--pr-context "${ARTIFACT_DIR}/pr-context.json"' in memory_block
    assert '--repo-path "${REPO_PATH}"' in memory_block
    assert f'--out "${{ARTIFACT_DIR}}/memory-context.md"' in memory_block
    for forbidden in (
        "cd pr-head", "python pr-head", "bash pr-head", "./pr-head", "pr-head/setup",
    ):
        assert forbidden not in memory_block


def test_context_composite_is_trusted_sourced_and_metadata_scoped():
    steps = _run_stage_steps()
    checkout = next(
        step for step in steps if _optional_str(step, "name") == "Checkout trusted workflow source"
    )
    context_step = _step_by_id(steps, "codex_context")
    action = _context_action_doc()
    runs = _required_map(action, "runs")

    _assert_trusted_source_resolver(steps, checkout)
    assert _required_str(checkout, "uses") == "actions/checkout@v4"
    assert _required_map(checkout, "with") == TRUSTED_SOURCE_CHECKOUT_WITH
    assert steps.index(checkout) < _step_index_by_id(steps, "codex_context")
    context_uses = _required_str(context_step, "uses")
    assert context_uses == "./trusted-core/.github/actions/codex-context"
    assert "pr-head" not in context_uses

    assert _required_str(runs, "using") == "composite"
    for step in _required_steps(runs, "steps"):
        if "run" in step:
            assert _optional_str(step, "shell")
            assert _optional_str(step, "working-directory")
    for required_input in (
        "workspace", "repo-path", "artifact-dir", "repository", "pr-number",
        "head-sha", "base-ref", "github-token",
    ):
        assert required_input in _required_map(action, "inputs")
    for output_name in (
        "pr_context_path", "changed_lines_path", "docs_context_path",
        "openspec_context_path", "memory_context_path",
    ):
        assert output_name in _required_map(action, "outputs")
    action_text = _context_action_text()
    assert "${{ secrets." not in action_text
    assert "\njobs:" not in action_text
    assert "\nneeds:" not in action_text


def test_review_phase_composite_is_trusted_sourced_and_metadata_scoped():
    steps = _run_stage_steps()
    checkout = next(
        step for step in steps if _optional_str(step, "name") == "Checkout trusted workflow source"
    )
    review_step = _step_by_id(steps, "review_complete")
    review_with = _required_map(review_step, "with")
    action = _review_action_doc()
    runs = _required_map(action, "runs")

    assert steps.index(checkout) < _step_index_by_id(steps, "review_complete")
    review_uses = _required_str(review_step, "uses")
    assert review_uses == "./trusted-core/.github/actions/codex-review-phase"
    assert review_uses != "./.github/actions/codex-review-phase"
    assert "pr-head" not in review_uses
    assert review_with == {
        "workspace": "${{ github.workspace }}",
        "repo-path": "pr-head",
        "common-artifact-dir": "codex-review-artifacts/common",
        "review-artifact-dir": "codex-review-artifacts/review",
        "review-state-bundle-dir": "codex-review-artifacts/review-state-bundle",
        "memory-context": MEMORY_CONTEXT,
        "relay-api-key": "${{ steps.relay_key.outputs.key }}",
        "codex-home-root": "${{ runner.temp }}/codex-home",
    }

    assert _required_str(runs, "using") == "composite"
    for step in _required_steps(runs, "steps"):
        if "run" in step:
            assert _optional_str(step, "shell")
            assert _optional_str(step, "working-directory")
    for required_input in (
        "workspace", "repo-path", "common-artifact-dir", "review-artifact-dir",
        "review-state-bundle-dir", "memory-context", "relay-api-key", "codex-home-root",
    ):
        assert required_input in _required_map(action, "inputs")
    for output_name in (
        "next_stage", "lgtm", "terminal_reason", "techlead_decision_path",
        "review_publication_path", "combined_findings_path", "route_path", "publish_report_path",
    ):
        assert output_name in _required_map(action, "outputs")
    outputs = _required_map(action, "outputs")
    assert _required_str(_required_map(outputs, "next_stage"), "value") == "${{ steps.review_complete.outputs.next_stage }}"
    assert _required_str(_required_map(outputs, "lgtm"), "value") == "${{ steps.review_complete.outputs.lgtm }}"
    assert _required_str(_required_map(outputs, "terminal_reason"), "value") == "${{ steps.review_complete.outputs.terminal_reason }}"
    action_text = _review_action_text()
    assert "${{ secrets." not in action_text
    assert "\njobs:" not in action_text
    assert "\nneeds:" not in action_text
    assert "pr-head/.github/actions" not in action_text


def test_fix_phase_composite_is_trusted_sourced_and_metadata_scoped():
    steps = _run_stage_steps()
    checkout = next(
        step for step in steps if _optional_str(step, "name") == "Checkout trusted workflow source"
    )
    loop_pat = _step_by_id(steps, "loop_pat")
    fix_step = _step_by_id(steps, "fix_complete")
    fix_with = _required_map(fix_step, "with")
    action = _fix_action_doc()
    runs = _required_map(action, "runs")

    assert steps.index(checkout) < _step_index_by_id(steps, "fix_complete")
    assert _step_index_by_id(steps, "loop_pat") < _step_index_by_id(steps, "fix_complete")
    assert _optional_str(loop_pat, "if") == "${{ steps.design_complete.outputs.next_stage == 'fix' }}"
    loop_pat_run = _required_str(loop_pat, "run")
    assert "${CODEX_LOOP_PAT:-}" in loop_pat_run
    assert 'echo "::add-mask::${loop_pat}"' in loop_pat_run
    fix_uses = _required_str(fix_step, "uses")
    assert fix_uses == "./trusted-core/.github/actions/codex-fix-phase"
    assert fix_uses != "./.github/actions/codex-fix-phase"
    assert "pr-head" not in fix_uses
    assert _optional_str(fix_step, "if") == "${{ steps.design_complete.outputs.next_stage == 'fix' }}"
    assert fix_with == {
        "workspace": "${{ github.workspace }}",
        "repo-path": "pr-head",
        "common-artifact-dir": "codex-review-artifacts/common",
        "design-state-bundle-dir": "codex-review-artifacts/design-state-bundle",
        "fix-dispatch-artifact-dir": "codex-review-artifacts/fix_dispatch",
        "fix-context-dir": "codex-review-artifacts/fix-context",
        "fix-merge-artifact-dir": "codex-review-artifacts/fix_merge",
        "fix-state-bundle-dir": "codex-review-artifacts/fix-state-bundle",
        "push-artifact-dir": "codex-review-artifacts/push",
        "push-state-bundle-dir": "codex-review-artifacts/push-state-bundle",
        "memory-context": MEMORY_CONTEXT,
        "loop-state-path": "${{ runner.temp }}/codex-loop-state.json",
        "relay-api-key": "${{ steps.relay_key.outputs.key }}",
        "loop-pat": "${{ steps.loop_pat.outputs.token }}",
        "codex-home-root": "${{ runner.temp }}/codex-home",
    }

    assert _required_str(runs, "using") == "composite"
    for step in _required_steps(runs, "steps"):
        if "run" in step:
            assert _optional_str(step, "shell")
            assert _optional_str(step, "working-directory")
    for required_input in (
        "workspace", "repo-path", "common-artifact-dir", "design-state-bundle-dir",
        "fix-dispatch-artifact-dir", "fix-context-dir", "fix-merge-artifact-dir",
        "fix-state-bundle-dir", "push-artifact-dir", "push-state-bundle-dir",
        "memory-context", "loop-state-path", "relay-api-key", "loop-pat", "codex-home-root",
    ):
        assert required_input in _required_map(action, "inputs")
    for output_name in (
        "next_stage", "lgtm", "terminal_reason", "should_redispatch",
        "push_terminal_reason", "updated_head_sha",
    ):
        assert output_name in _required_map(action, "outputs")
    outputs = _required_map(action, "outputs")
    assert _required_str(_required_map(outputs, "next_stage"), "value") == "${{ steps.fix_complete.outputs.next_stage }}"
    assert _required_str(_required_map(outputs, "terminal_reason"), "value") == "${{ steps.fix_complete.outputs.terminal_reason }}"
    assert _required_str(_required_map(outputs, "should_redispatch"), "value") == "${{ steps.push_complete.outputs.should_redispatch }}"
    assert _required_str(_required_map(outputs, "push_terminal_reason"), "value") == "${{ steps.push_complete.outputs.terminal_reason }}"
    assert _required_str(_required_map(outputs, "updated_head_sha"), "value") == "${{ steps.push_complete.outputs.updated_head_sha }}"
    action_text = _fix_action_text()
    assert "${{ secrets." not in action_text
    assert "\njobs:" not in action_text
    assert "\nneeds:" not in action_text
    assert "pr-head/.github/actions" not in action_text


def test_review_phase_composite_model_steps_preserve_codex_action_shape():
    model_steps = [
        step for step in _review_action_steps()
        if _optional_str(step, "uses") == MODEL_ACTION
    ]
    assert [_required_str(step, "name") for step in model_steps] == [
        "Run live Codex review findings",
        "Run live Codex review techlead",
    ]
    expected = {
        "Run live Codex review findings": {
            "if": "${{ steps.prepare_stage.outputs.needs_review_axis_model == 'true' }}",
            "codex-home": "${{ inputs.codex-home-root }}/review-findings",
            "output-file": "${{ inputs.workspace }}/${{ inputs.review-artifact-dir }}/correctness/findings.raw.json",
            "output-schema-file": "${{ inputs.workspace }}/${{ inputs.review-artifact-dir }}/correctness/findings.strict.json",
            "prompt-file": "${{ inputs.workspace }}/${{ inputs.review-artifact-dir }}/correctness/prompt.md",
            "schema": "review-axis-findings.v1",
        },
        "Run live Codex review techlead": {
            "if": "${{ steps.review_techlead_prepare.outputs.needs_review_techlead_model == 'true' }}",
            "codex-home": "${{ inputs.codex-home-root }}/review-techlead",
            "output-file": "${{ inputs.workspace }}/${{ inputs.review-artifact-dir }}/techlead-decision.raw.json",
            "output-schema-file": "${{ inputs.workspace }}/${{ inputs.review-artifact-dir }}/techlead-decision.strict.json",
            "prompt-file": "${{ inputs.workspace }}/${{ inputs.review-artifact-dir }}/techlead.prompt.md",
            "schema": "techlead-decision.v1",
        },
    }
    homes: list[str] = []
    for step in model_steps:
        step_name = _required_str(step, "name")
        w = _optional_map(step, "with")
        assert _optional_str(step, "if") == expected[step_name]["if"]
        assert _optional_str(w, "openai-api-key") == "${{ inputs.relay-api-key }}"
        assert _optional_str(w, "responses-api-endpoint") == RELAY_ENDPOINT
        assert _optional_str(w, "effort") == "xhigh"
        assert _optional_str(w, "safety-strategy") == "unsafe"
        assert _optional_str(w, "sandbox") == "danger-full-access"
        assert _optional_str(w, "working-directory") == "pr-head"
        assert _optional_str(w, "codex-home") == expected[step_name]["codex-home"]
        assert _optional_str(w, "output-file") == expected[step_name]["output-file"]
        assert _optional_str(w, "output-schema-file") == expected[step_name]["output-schema-file"]
        prompt = _required_str(w, "prompt")
        assert expected[step_name]["prompt-file"] in prompt
        assert expected[step_name]["schema"] in prompt
        homes.append(_optional_str(w, "codex-home"))
    assert len(homes) == len(set(homes)) == 2


def test_prompt_builders_receive_common_memory_context():
    steps = _run_stage_steps()
    review_with = _required_map(_step_by_id(steps, "review_complete"), "with")
    design_with = _required_map(_step_by_id(steps, "design_complete"), "with")
    fix_with = _required_map(_step_by_id(steps, "fix_complete"), "with")
    assert _required_str(review_with, "memory-context") == MEMORY_CONTEXT
    assert _required_str(design_with, "memory-context") == MEMORY_CONTEXT
    assert _required_str(fix_with, "memory-context") == MEMORY_CONTEXT

    for step_id, command in REVIEW_PROMPT_BUILDERS_WITH_MEMORY:
        step = _step_by_id(_review_action_steps(), step_id)
        env = _required_map(step, "env")
        block = _command_block(_required_str(step, "run"), command)
        assert _required_str(env, "MEMORY_CONTEXT") == "${{ inputs.memory-context }}"
        assert '--memory-context "${MEMORY_CONTEXT}"' in block, (step_id, block)
        assert block.count("--memory-context") == 1, (step_id, block)

    for step_id, command in DESIGN_PROMPT_BUILDERS_WITH_MEMORY:
        step = _step_by_id(_design_action_steps(), step_id)
        env = _required_map(step, "env")
        block = _command_block(_required_str(step, "run"), command)
        assert _required_str(env, "MEMORY_CONTEXT") == "${{ inputs.memory-context }}"
        assert '--memory-context "${MEMORY_CONTEXT}"' in block, (step_id, block)
        assert block.count("--memory-context") == 1, (step_id, block)

    for step_id, command in FIX_PROMPT_BUILDERS_WITH_MEMORY:
        step = _step_by_id(_fix_action_steps(), step_id)
        env = _required_map(step, "env")
        block = _command_block(_required_str(step, "run"), command)
        assert _required_str(env, "MEMORY_CONTEXT") == "${{ inputs.memory-context }}"
        assert '--memory-context "${MEMORY_CONTEXT}"' in block, (step_id, block)
        assert block.count("--memory-context") == 1, (step_id, block)

    assert _design_action_text().count('--memory-context "${MEMORY_CONTEXT}"') == len(
        DESIGN_PROMPT_BUILDERS_WITH_MEMORY
    )
    assert _fix_action_text().count('--memory-context "${MEMORY_CONTEXT}"') == len(
        FIX_PROMPT_BUILDERS_WITH_MEMORY
    )
    assert len(PROMPT_BUILDERS_WITH_MEMORY) == 9


def test_no_stage_input_remains():
    call_inputs = _workflow_call_inputs()
    # The single-run loop no longer selects a stage; the whole chain runs in
    # one run. Cross-run state pointers are gone too.
    assert "stage" not in call_inputs, call_inputs
    assert "state_run_id" not in call_inputs, call_inputs
    assert "state_artifact_name" not in call_inputs, call_inputs
    # The entry-point inputs the next iteration needs must remain.
    for required in ("pr_number", "head_sha", "base_ref", "iteration",
                     "correlation_id", "requested_by"):
        assert required in call_inputs, required
    # `inputs.stage` must not be referenced anywhere.
    assert "inputs.stage" not in _text()


def test_no_cross_run_state_machinery_remains():
    text = _text()
    # These tokens only existed because stages were separate runs. In a single
    # sequential run, each stage reads the prior stage's in-run bundle.
    for token in (
        "verify-bundle",
        "prior-state",
        "Download prior stage state artifact",
        "Download prior loop state artifact",
        "${RUNNER_TEMP}/codex-loop-stage-result.json",
        ".codex-loop-stage-result.json",
    ):
        assert token not in text, f"cross-run machinery still present: {token}"


def test_no_deleted_machinery_remains():
    text = _text()
    for token in (
        "oidc", "relay-token", "relay_configured", "live_ready",
        "trust-and-stale-guard", "eligible", "auth app-token", "app_token",
        "CODEX_GITHUB_APP", "max_iterations", "guard-dispatch",
        "append-dispatch-ledger", "id-token",
    ):
        assert token not in text, f"deleted machinery still present: {token}"


def test_sequential_stage_gating_on_prior_completion():
    steps = _run_stage_steps()
    # Each completion surface exposes the next stage as a step output.
    review_complete = _step_by_id(steps, "review_complete")
    assert _required_str(review_complete, "uses") == "./trusted-core/.github/actions/codex-review-phase"
    review_outputs = _required_map(_review_action_doc(), "outputs")
    assert _required_str(_required_map(review_outputs, "next_stage"), "value") == "${{ steps.review_complete.outputs.next_stage }}"
    assert _required_str(_required_map(review_outputs, "lgtm"), "value") == "${{ steps.review_complete.outputs.lgtm }}"
    assert _required_str(_required_map(review_outputs, "terminal_reason"), "value") == "${{ steps.review_complete.outputs.terminal_reason }}"

    design_complete = _step_by_id(steps, "design_complete")
    assert _required_str(design_complete, "uses") == "./trusted-core/.github/actions/codex-design-phase"
    assert _optional_str(design_complete, "if") == "${{ steps.review_complete.outputs.next_stage == 'design' }}"
    assert _required_map(design_complete, "with") == {
        "workspace": "${{ github.workspace }}",
        "repo-path": "pr-head",
        "common-artifact-dir": "codex-review-artifacts/common",
        "review-state-bundle-dir": "codex-review-artifacts/review-state-bundle",
        "design-artifact-dir": "codex-review-artifacts/design",
        "design-state-bundle-dir": "codex-review-artifacts/design-state-bundle",
        "memory-context": MEMORY_CONTEXT,
        "relay-api-key": "${{ steps.relay_key.outputs.key }}",
        "codex-home-root": "${{ runner.temp }}/codex-home",
    }
    design_outputs = _required_map(_design_action_doc(), "outputs")
    assert _required_str(_required_map(design_outputs, "next_stage"), "value") == "${{ steps.design_complete.outputs.next_stage }}"
    assert _required_str(_required_map(design_outputs, "lgtm"), "value") == "${{ steps.design_complete.outputs.lgtm }}"
    assert _required_str(_required_map(design_outputs, "terminal_reason"), "value") == "${{ steps.design_complete.outputs.terminal_reason }}"
    design_complete_run = _design_action_run("design_complete")
    assert 'echo "next_stage=${next_stage}"' in design_complete_run
    assert 'echo "lgtm=${lgtm}"' in design_complete_run
    assert 'echo "terminal_reason=${terminal_reason}"' in design_complete_run

    fix_complete = _step_by_id(steps, "fix_complete")
    assert _required_str(fix_complete, "uses") == "./trusted-core/.github/actions/codex-fix-phase"
    assert _optional_str(fix_complete, "if") == "${{ steps.design_complete.outputs.next_stage == 'fix' }}"
    fix_outputs = _required_map(_fix_action_doc(), "outputs")
    assert _required_str(_required_map(fix_outputs, "next_stage"), "value") == "${{ steps.fix_complete.outputs.next_stage }}"
    assert _required_str(_required_map(fix_outputs, "lgtm"), "value") == "${{ steps.fix_complete.outputs.lgtm }}"
    assert _required_str(_required_map(fix_outputs, "terminal_reason"), "value") == "${{ steps.fix_complete.outputs.terminal_reason }}"
    assert _required_str(_required_map(fix_outputs, "should_redispatch"), "value") == "${{ steps.push_complete.outputs.should_redispatch }}"
    assert _required_str(_required_map(fix_outputs, "push_terminal_reason"), "value") == "${{ steps.push_complete.outputs.terminal_reason }}"
    assert _required_str(_required_map(fix_outputs, "updated_head_sha"), "value") == "${{ steps.push_complete.outputs.updated_head_sha }}"
    fix_complete_run = _fix_action_run("fix_complete")
    assert 'echo "next_stage=${next_stage}"' in fix_complete_run
    assert 'echo "lgtm=${lgtm}"' in fix_complete_run
    assert 'echo "terminal_reason=${terminal_reason}"' in fix_complete_run

    # Review steps are the always-on first stage: they never gate on a prior
    # stage's next_stage output.
    review_complete_if = _optional_str(review_complete, "if")
    assert "next_stage" not in review_complete_if, review_complete_if

    # Design is a single trusted workflow-level composite call gated on review.
    assert _step_index_by_id(steps, "review_complete") < _step_index_by_id(steps, "design_complete")

    # Fix is a single trusted workflow-level composite call gated on design.
    assert _step_index_by_id(steps, "design_complete") < _step_index_by_id(steps, "fix_complete")

    # Push steps gate inside the fix composite on the fix completion's next_stage == push.
    push_gate = "steps.fix_complete.outputs.next_stage == 'push'"
    for step_id in ("push_commit", "push_complete"):
        cond = _optional_str(_step_by_id(_fix_action_steps(), step_id), "if")
        assert push_gate in cond, (step_id, cond)


def test_design_fix_push_read_in_run_bundles_not_prior_state():
    # Design reads the review bundle's techlead-decision inside its composite.
    design_prep = _design_action_run("design_inventory_prepare")
    assert "${REVIEW_STATE_BUNDLE_DIR}/techlead-decision.json" in design_prep
    # Fix reads the design bundle's plan + chief decision inside its composite.
    fix_prep = _fix_action_run("fix_dispatch_prepare")
    assert "${DESIGN_STATE_BUNDLE_DIR}/design-plan.json" in fix_prep
    assert "${DESIGN_STATE_BUNDLE_DIR}/chief-decision.json" in fix_prep
    # Push reads the fix bundle's merged/validated fix inside the fix composite.
    push = _fix_action_run("push_commit")
    assert "${FIX_STATE_BUNDLE_DIR}/merged-fix.json" in push
    assert "${FIX_STATE_BUNDLE_DIR}/validated-fix.json" in push


def test_no_workflow_call_secrets_and_no_secret_refs():
    # Credentials come from runner env vars, not GitHub secrets.
    assert "secrets" not in _workflow_call(), "core must declare no workflow_call secrets"
    assert "${{ secrets." not in _text(), "no secrets.* references allowed"


def test_relay_key_read_from_runner_env_and_fed_to_model():
    text = "\n".join((_text(), _review_action_text(), _design_action_text(), _fix_action_text()))
    # A capture step reads the runner env var (fail-fast) and exposes it.
    assert "${CODEX_RELAY_API_KEY:?" in text
    assert 'echo "::add-mask::${CODEX_RELAY_API_KEY}"' in text
    seen = 0
    structured = 0
    fix_agents_seen = False
    homes: list[str] = []
    model_sources: tuple[tuple[str, YamlSteps, str, str, str], ...] = (
        (
            "review composite",
            _review_action_steps(),
            "${{ inputs.relay-api-key }}",
            "${{ inputs.codex-home-root }}/",
            "pr-head",
        ),
        (
            "design composite",
            _design_action_steps(),
            "${{ inputs.relay-api-key }}",
            "${{ inputs.codex-home-root }}/",
            "pr-head",
        ),
        (
            "fix composite",
            _fix_action_steps(),
            "${{ inputs.relay-api-key }}",
            "${{ inputs.codex-home-root }}/",
            "${{ inputs.repo-path }}",
        ),
    )
    for source_name, steps, expected_api_key, expected_home_prefix, expected_working_dir in model_sources:
        for step in steps:
            if _optional_str(step, "uses") == MODEL_ACTION:
                seen += 1
                step_name = _optional_str(step, "name")
                label = f"{source_name}: {step_name}"
                w = _optional_map(step, "with")
                assert _optional_str(w, "openai-api-key") == expected_api_key, label
                assert _optional_str(w, "responses-api-endpoint") == RELAY_ENDPOINT, label
                # gpt-5.5 defaults to medium effort; the loop runs every model
                # step at the highest reasoning tier.
                assert _optional_str(w, "effort") == "xhigh", label
                # Container is the isolation boundary; the action's sudo-drop
                # sandbox needs passwordless sudo the runner doesn't grant.
                assert _optional_str(w, "safety-strategy") == "unsafe", label
                # Runner container can't nest user namespaces, so codex must not
                # use bubblewrap — run commands directly (full access).
                assert _optional_str(w, "sandbox") == "danger-full-access", label
                assert _optional_str(w, "working-directory") == expected_working_dir, label
                # Fresh per-step Codex home (persistent runner would otherwise
                # accumulate duplicate keys in ~/.codex/config.toml).
                ch = _optional_str(w, "codex-home")
                assert ch.startswith(expected_home_prefix), label
                homes.append(ch)
                if not step_name.startswith("Run live Codex"):
                    continue
                if "fix agents" in step_name:
                    # Multi-file emitter writes agents/*/result.json itself.
                    fix_agents_seen = True
                else:
                    # Single-JSON steps capture deterministic output via the
                    # codex-action structured-output contract.
                    assert _optional_str(w, "output-file"), f"missing output-file: {label}"
                    assert _optional_str(w, "output-schema-file"), (
                        f"missing output-schema-file: {label}"
                    )
                    structured += 1
    assert seen >= 9, f"expected >=9 model steps, saw {seen}"
    assert structured >= 8, f"expected >=8 structured-output steps, saw {structured}"
    assert fix_agents_seen, "fix agents step not found"
    # Per-step codex-home must be unique so invocations never share config.toml.
    assert len(homes) == len(set(homes)) == seen, homes
    # Each structured step emits its OpenAI strict schema before running.
    assert text.count("schema openai-strict") >= 8


def test_push_and_dispatch_use_the_pat_from_runner_env():
    text = "\n".join((_text(), _fix_action_text()))
    assert "${CODEX_LOOP_PAT:-}" in text
    assert "${CODEX_LOOP_PAT:?" in text
    assert '--token "${CODEX_LOOP_PAT}"' in text
    assert 'export GH_TOKEN="${CODEX_LOOP_PAT}"' in text


def test_loop_scratch_lives_under_runner_temp():
    # Self-hosted runners reuse the workspace between jobs but wipe RUNNER_TEMP
    # per job. The loop-state file is per-job scratch; keeping it under
    # RUNNER_TEMP makes cross-job leakage impossible by construction. Mirrors
    # the codex-home pattern already used by the model steps.
    text = _text()
    # The loop-state file is never read/written/uploaded from the workspace root.
    for bare in (
        "--out codex-loop-state.json",
        "--loop-state codex-loop-state.json",
        "path: codex-loop-state.json",
    ):
        assert bare not in text, bare
    assert '--out "${RUNNER_TEMP}/codex-loop-state.json"' in text
    assert "${{ runner.temp }}/codex-loop-state.json" in text
    assert "Reset stale loop workspace scratch" not in text


def test_setup_state_bootstraps_fresh_each_iteration():
    # No cross-run state-pointer download; each iteration bootstraps a fresh
    # loop-state with read-state and no --loop-state argument.
    setup = _required_steps(_job("setup-state"), "steps")
    boot = _required_str(_step_by_id(setup, "state"), "run")
    assert "codex-review loop read-state --out" in boot
    assert "--loop-state" not in boot


def test_continuation_dispatch_payload_has_no_stage_or_state():
    text = _text()
    resolve = _required_str(_step_by_id(_required_steps(_job("finalize"), "steps"), "resolve"), "run")
    # The next-iteration payload must NOT carry a stage or state pointers.
    assert "$stage" not in resolve, resolve
    assert "state_run_id" not in resolve, resolve
    assert "state_artifact_name" not in resolve, resolve
    # It MUST carry the next iteration's entry inputs.
    for key in ("pr_number", "head_sha", "base_ref", "iteration",
                "correlation_id", "requested_by"):
        assert f"${key}" in resolve or f"--arg {key}" in resolve, key
    # event_type stays codex-loop.
    assert '--arg event_type "codex-loop"' in text


def test_continuation_dispatch_gates_only_on_dispatch_candidate():
    finalize_steps = _required_steps(_job("finalize"), "steps")
    dispatch_ifs = [
        _optional_str(step, "if")
        for step in finalize_steps
        if "dispatches" in _optional_str(step, "run")
        or "repository_dispatch" in _optional_str(step, "name").lower()
    ]
    assert dispatch_ifs, "expected a continuation dispatch step"
    for cond in dispatch_ifs:
        assert cond == "${{ steps.resolve.outputs.dispatch_candidate == 'true' }}", cond


def test_classify_gate_precedes_and_gates_model_stage():
    jobs = _jobs()
    assert list(jobs).index("classify") < list(jobs).index("run-stage")

    classify = jobs["classify"]
    outputs = _required_map(classify, "outputs")
    assert _required_str(outputs, "memory_only") == "${{ steps.classify.outputs.memory_only }}"
    assert _required_str(outputs, "should_run_model") == "${{ steps.classify.outputs.should_run_model }}"
    assert _required_str(outputs, "codex_memory_marker") == "${{ steps.classify.outputs.codex_memory_marker }}"
    assert _required_str(outputs, "actor_guard") == "${{ steps.classify.outputs.actor_guard }}"
    classify_steps = _required_steps(classify, "steps")
    checkout = next(
        step for step in classify_steps if _optional_str(step, "name") == "Checkout trusted workflow source"
    )
    classify_step = _step_by_id(classify_steps, "classify")
    classify_with = _required_map(classify_step, "with")
    _assert_trusted_source_resolver(classify_steps, checkout)
    assert _required_str(checkout, "uses") == "actions/checkout@v4"
    assert _required_map(checkout, "with") == TRUSTED_SOURCE_CHECKOUT_WITH
    assert classify_steps.index(checkout) < _step_index_by_id(classify_steps, "classify")
    assert _required_str(classify_step, "uses") == "./trusted-core/.github/actions/codex-memory-classify"
    assert classify_with == {
        "workspace": "${{ github.workspace }}",
        "repo-path": "pr-head",
        "head-sha": "${{ inputs.head_sha }}",
        "github-actor": "${{ github.actor }}",
        "requested-by": "${{ inputs.requested_by }}",
        "own-actors": "${{ vars.CODEX_LOOP_OWN_ACTORS || 'github-actions[bot]' }}",
        "output-path": "${{ runner.temp }}/codex-memory-only-change.json",
    }

    action = _memory_action_doc()
    runs = _required_map(action, "runs")
    action_outputs = _required_map(action, "outputs")
    classify_run = _memory_action_run("classify")
    summary_run = _required_str(
        next(
            step
            for step in _memory_action_steps()
            if _optional_str(step, "name") == "Summarize memory-only classification"
        ),
        "run",
    )
    assert _required_str(runs, "using") == "composite"
    for step in _required_steps(runs, "steps"):
        if "run" in step:
            assert _optional_str(step, "shell")
            assert _optional_str(step, "working-directory")
    for required_input in (
        "workspace", "repo-path", "head-sha", "github-actor", "requested-by",
        "own-actors", "output-path",
    ):
        assert required_input in _required_map(action, "inputs")
    for output_name in (
        "memory_only", "should_run_model", "classification_reason",
        "codex_memory_marker", "actor_guard",
    ):
        assert output_name in action_outputs
        assert _required_str(_required_map(action_outputs, output_name), "value") == (
            f"${{{{ steps.classify.outputs.{output_name} }}}}"
        )
    assert "codex-review loop memory-only-change" in classify_run
    assert '--repo-path "${REPO_PATH}"' in classify_run
    assert "--base" in classify_run
    assert "--head" in classify_run
    classify_env = _required_map(_step_by_id(_memory_action_steps(), "classify"), "env")
    assert _required_str(classify_env, "CODEX_LOOP_GITHUB_ACTOR") == "${{ inputs.github-actor }}"
    assert _required_str(classify_env, "CODEX_LOOP_REQUESTED_BY") == "${{ inputs.requested-by }}"
    assert _required_str(classify_env, "CODEX_LOOP_OWN_ACTORS") == "${{ inputs.own-actors }}"
    assert "codex-memory marker=" in summary_run
    assert "actor_guard=" in summary_run
    assert MODEL_ACTION not in str(classify)
    action_text = _memory_action_text()
    assert "${{ secrets." not in action_text
    assert "\njobs:" not in action_text
    assert "\nneeds:" not in action_text
    assert "pr-head/.github/actions" not in action_text

    run_stage = jobs["run-stage"]
    assert "needs.classify.outputs.should_run_model == 'true'" in _required_str(run_stage, "if")
    assert _required_str_list(run_stage, "needs") == ["validate", "setup-state", "classify"]
    for job_name, job in jobs.items():
        for step in _optional_steps(job, "steps"):
            if _optional_str(step, "uses") == MODEL_ACTION:
                assert job_name == "run-stage"


def test_finalize_emits_memory_only_noop_without_redispatch():
    finalize = _job("finalize")
    assert _required_str_list(finalize, "needs") == ["validate", "setup-state", "classify", "run-stage"]
    resolve = _step_by_id(_required_steps(finalize, "steps"), "resolve")
    env = _required_map(resolve, "env")
    assert _required_str(env, "CLASSIFY_MEMORY_ONLY") == "${{ needs.classify.outputs.memory_only }}"
    assert _required_str(env, "CLASSIFY_SHOULD_RUN_MODEL") == "${{ needs.classify.outputs.should_run_model }}"
    assert _required_str(env, "CLASSIFY_CODEX_MEMORY_MARKER") == "${{ needs.classify.outputs.codex_memory_marker }}"
    assert _required_str(env, "CLASSIFY_ACTOR_GUARD") == "${{ needs.classify.outputs.actor_guard }}"
    run = _required_str(resolve, "run")
    assert '"${CLASSIFY_CODEX_MEMORY_MARKER}" == "true"' in run
    assert '"${CLASSIFY_ACTOR_GUARD}" == "true"' in run
    assert 'terminal_reason="memory_only_noop"' in run
    assert 'should_redispatch="false"' in run
    assert 'updated_head_sha=""' in run


def test_required_checks_aggregates_all_jobs_and_allows_skips():
    required = _job("required-checks")
    assert _required_str_list(required, "needs") == ["validate", "setup-state", "classify", "run-stage", "finalize"]
    assert _required_str(required, "if") == "${{ !cancelled() }}"
    run = _required_str(_required_steps(required, "steps")[0], "run")
    assert '.result == "failure" or .result == "cancelled"' in run
    assert 'exit 1' in run
    assert 'skipped' not in run.split('exit 1')[0]
