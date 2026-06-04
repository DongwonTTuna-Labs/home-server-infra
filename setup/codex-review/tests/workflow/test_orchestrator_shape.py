"""Shape/topology contract for the Codex loop reusable-core pipeline.

The reusable core is a single entrypoint with common validation/trust/topology
setup followed by one explicitly gated stage skeleton for each supported stage.
Stage internals are intentionally deferred to later tasks; this file preserves the
trust-boundary and output-normalization shape required before those internals land.
"""
import re
from pathlib import Path

from _pipeline import (
    DISPATCH,
    MANUAL,
    RESPONSES_ENDPOINT,
    REUSABLE,
    all_text,
    load,
    reusable_jobs,
    workflow_path,
)

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"

REVIEW_STAGE_JOBS = ("review-collect", "review-axes", "review-combine", "review-techlead")
DESIGN_STAGE_JOBS = ("design-prepare", "design-analyze", "design-plan", "design-chief")
FIX_STAGE_JOBS = ("fix-dispatch", "fix-run-agents", "fix-merge-validate", "fix-push")
ISSUE_STAGE_JOBS = ("issue-stage",)
FINALIZE_OUTPUT_KEYS = (
    "next_stage",
    "lgtm",
    "should_redispatch",
    "terminal_reason",
    "state_run_id",
    "state_artifact_name",
    "updated_head_sha",
)
ALL_STAGE_JOBS = (*REVIEW_STAGE_JOBS, *DESIGN_STAGE_JOBS, *FIX_STAGE_JOBS, *ISSUE_STAGE_JOBS)
REVIEW_OUTPUT_JOB = "review-techlead"
DESIGN_OUTPUT_JOB = "design-chief"
FIX_OUTPUT_JOB = "fix-merge-validate"
REVIEW_AXES = ("correctness", "security", "performance", "test-coverage", "domain")
REUSABLE_JOB_ORDER = [
    "validate",
    "trust-and-stale-guard",
    "setup-relay",
    *ALL_STAGE_JOBS,
    "finalize-stage",
]
LEGACY_SPLIT_WORKFLOWS = (
    "codex-review.yml",
    "codex-design.yml",
    "codex-fix.yml",
    "codex-issue.yml",
    "codex-review-orchestrator.yml",
    "resolve-checker.yml",
)


def reusable_text() -> str:
    return REUSABLE.read_text(encoding="utf-8")


def test_codex_loop_pipeline_workflow_files_present():
    workflows = list(WORKFLOWS.glob("*.yml")) + list(WORKFLOWS.glob("*.yaml"))
    names = {p.name for p in workflows}
    assert {
        "codex-loop-reusable.yml",
        "codex-loop-dispatch.yml",
        "codex-loop-manual.yml",
    } <= names
    for legacy in LEGACY_SPLIT_WORKFLOWS:
        assert legacy not in names, legacy


def test_reusable_core_declares_expected_job_order():
    jobs = list(reusable_jobs().keys())
    assert jobs == REUSABLE_JOB_ORDER


def test_reusable_core_serializes_per_loop_without_cancelling():
    workflow = load(REUSABLE)
    assert workflow["concurrency"]["group"] == "codex-loop-${{ inputs.correlation_id }}"
    assert workflow["concurrency"]["cancel-in-progress"] is False


def test_dispatch_adapter_serializes_per_loop_without_cancelling():
    workflow = load(DISPATCH)
    assert (
        workflow["concurrency"]["group"]
        == "codex-loop-${{ github.event.client_payload.correlation_id }}"
    )
    assert workflow["concurrency"]["cancel-in-progress"] is False


def test_reusable_and_dispatch_concurrency_share_correlation_id_key():
    reusable = load(REUSABLE)
    dispatch = load(DISPATCH)
    assert reusable["concurrency"]["group"] == "codex-loop-${{ inputs.correlation_id }}"
    assert dispatch["concurrency"]["group"] == "codex-loop-${{ github.event.client_payload.correlation_id }}"
    assert reusable["concurrency"]["group"].startswith("codex-loop-")
    assert dispatch["concurrency"]["group"].startswith("codex-loop-")
    assert "correlation_id" in reusable["concurrency"]["group"]
    assert "correlation_id" in dispatch["concurrency"]["group"]


def test_no_inline_python_or_schema_bloat():
    text = all_text()
    assert "python - <<" not in text
    assert "python3 - <<" not in text
    assert "json-schema.org" not in text
    assert "bin/codex-review" not in text


def test_no_generic_prompt_placeholder_or_error_suppression():
    text = reusable_text()
    assert "Run Codex loop stage" not in text
    assert "dry-run-placeholder" not in text
    assert "live-stage-placeholder" not in text
    assert "Parse review stage result JSON" in text
    assert "missing-stage-result-json" in text
    assert "jq -er '.next_stage | strings'" in text
    assert "jq -er '.lgtm | booleans | tostring'" in text
    assert "jq -er '.should_redispatch | booleans | tostring'" in text
    assert "jq -er '.terminal_reason | strings'" in text


def test_reusable_routes_stages_via_stage_input_gates():
    jobs = reusable_jobs()
    assert "inputs.stage == 'review'" in jobs["review-collect"]["if"]
    assert jobs["review-collect"]["needs"] == ["validate", "trust-and-stale-guard", "setup-relay"]
    for review_job in REVIEW_STAGE_JOBS:
        assert "inputs.stage == 'review'" in jobs[review_job]["if"]
    for design_job in DESIGN_STAGE_JOBS:
        assert "inputs.stage == 'design'" in jobs[design_job]["if"]
    assert jobs["design-prepare"]["needs"] == ["validate", "trust-and-stale-guard", "setup-relay"]
    assert jobs["fix-dispatch"]["needs"] == ["validate", "trust-and-stale-guard", "setup-relay"]
    for fix_job in FIX_STAGE_JOBS:
        assert "inputs.stage == 'fix'" in jobs[fix_job]["if"]
    assert jobs["fix-run-agents"]["needs"] == ["setup-relay", "fix-dispatch"]
    assert jobs["fix-merge-validate"]["needs"] == ["setup-relay", "fix-dispatch", "fix-run-agents"]
    assert jobs["fix-push"]["needs"] == ["validate", "trust-and-stale-guard", "setup-relay", "fix-merge-validate"]
    assert "inputs.stage == 'issue'" in jobs["issue-stage"]["if"]
    assert jobs["issue-stage"]["needs"] == ["validate", "trust-and-stale-guard", "setup-relay"]
    text = reusable_text()
    assert 'next_stage="fix"' in text
    assert 'next_stage="review"' in text
    assert 'terminal_reason="artifact_missing"' in text
    assert 'next_stage="design"' in text


def test_checkout_credentials_are_never_persisted():
    text = all_text()
    assert text.count("actions/checkout@") == text.count("persist-credentials: false")
    assert text.count("actions/checkout@") >= 3


def test_stage_decision_json_is_validated_in_workflow():
    text = reusable_text()
    assert "invalid-next-stage" in text
    assert "redispatch-missing-next-stage" in text
    assert "terminal-reason-required" in text
    assert "review|design|fix|issue) ;;" in text


def test_workflows_declare_expected_triggers():
    assert "workflow_call:" in reusable_text()
    dispatch = DISPATCH.read_text(encoding="utf-8")
    assert "repository_dispatch:" in dispatch
    assert "types: [codex-loop]" in dispatch
    assert "workflow_dispatch:" in MANUAL.read_text(encoding="utf-8")


def test_adapters_thread_pr_identity_into_reusable_core():
    dispatch = DISPATCH.read_text(encoding="utf-8")
    manual = MANUAL.read_text(encoding="utf-8")
    assert "PAYLOAD_PR_NUMBER: ${{ github.event.client_payload.pr_number }}" in dispatch
    assert "pr_number: ${{ inputs.pr_number }}" in manual
    for field in ("pr_number", "head_sha", "base_ref", "stage", "correlation_id"):
        assert f"{field}: " in dispatch
        assert f"{field}: ${{{{ inputs.{field} }}}}" in manual


def test_setup_relay_downloads_and_reads_prior_loop_state():
    job = reusable_jobs()["setup-relay"]
    download_steps = [
        step
        for step in job.get("steps", [])
        if step.get("uses", "").startswith("actions/download-artifact@")
    ]
    assert len(download_steps) == 1
    with_inputs = download_steps[0]["with"]
    assert with_inputs["run-id"] == "${{ inputs.state_run_id }}"
    assert with_inputs["name"] == "${{ inputs.state_artifact_name }}"
    assert "codex-review loop read-state" in reusable_text()


def test_setup_relay_prior_state_bundle_download_is_non_blocking():
    # Regression: a hard-failing prior-state download made setup-relay fail before the
    # stage verify-bundle ran, normalizing missing bundles to stage_failed not artifact_missing.
    job = reusable_jobs()["setup-relay"]
    download = next(
        step
        for step in job.get("steps", [])
        if step.get("uses", "").startswith("actions/download-artifact@")
    )
    assert download.get("continue-on-error") is True
    assert download["with"]["run-id"] == "${{ inputs.state_run_id }}"
    assert download["with"]["name"] == "${{ inputs.state_artifact_name }}"


def test_stage_jobs_do_not_use_generic_model_prompt():
    jobs = reusable_jobs()
    text = reusable_text()
    assert "openai/codex-action@v1" not in text
    for job_name in ALL_STAGE_JOBS:
        steps = jobs[job_name].get("steps", [])
        assert all(step.get("uses") != "openai/codex-action@v1" for step in steps)
    assert "Fix Stage Skeleton" not in text
    assert "Issue Stage Skeleton" not in text
    assert "codex-review fix_dispatch plan" in text
    assert "codex-review issue_fallback plan" in text


def test_stage_skeletons_reserve_self_hosted_responses_endpoint_context():
    text = reusable_text()
    assert RESPONSES_ENDPOINT in text
    relay_context_jobs = (
        "review-collect", "review-axes", "review-techlead",
        *DESIGN_STAGE_JOBS, "fix-dispatch", "fix-run-agents", "fix-merge-validate", *ISSUE_STAGE_JOBS,
    )
    for job_name in relay_context_jobs:
        env_maps = [step.get("env", {}) for step in reusable_jobs()[job_name].get("steps", [])]
        assert any(
            env.get("RESPONSES_ENDPOINT")
            == "${{ needs.setup-relay.outputs.responses_endpoint }}"
            for env in env_maps
        ), job_name


def test_workflow_has_no_legacy_model_runner_env_contract():
    text = all_text()
    assert "CODEX_REVIEW_MODEL_COMMAND" not in text
    assert "CODEX_REVIEW_CODEX_ARGS_JSON" not in text
    assert "codex-review-model-runner" not in text


def test_finalize_stage_and_fix_push_are_the_only_repo_write_paths_in_core():
    jobs = reusable_jobs()
    for name, job in jobs.items():
        perms = job.get("permissions", {})
        if name == "finalize-stage":
            assert perms.get("contents") == "write"
            assert perms.get("pull-requests") == "write"
        else:
            assert perms.get("contents") != "write", name
            assert perms.get("pull-requests") != "write", name
    assert "codex-review auth app-token" in str(jobs["fix-push"])
    assert "--mode push" in str(jobs["fix-push"])
    assert "codex-review auth app-token" in str(jobs["finalize-stage"])
    assert "--mode dispatch" in str(jobs["finalize-stage"])


def test_model_and_validation_jobs_never_request_app_token():
    jobs = reusable_jobs()
    for name, job in jobs.items():
        if name in {"fix-push", "finalize-stage"}:
            continue
        assert "auth app-token" not in str(job), name


def test_finalize_stage_emits_live_repository_dispatch_with_app_token():
    finalize = reusable_jobs()["finalize-stage"]
    text = str(finalize)
    assert "Build continuation dispatch context and payload" in text
    assert "Guard continuation dispatch ledger" in text
    assert "Mint GitHub App installation token for dispatch" in text
    assert "Emit repository dispatch continuation" in text
    assert "codex-review auth app-token" in text
    assert "--mode dispatch" in text
    assert 'event_type: $event_type' in text
    assert '"codex-loop"' in text
    assert "repos/${REPOSITORY}/dispatches" in text
    assert "Authorization: Bearer ${DISPATCH_APP_TOKEN}" in text
    dispatch_step = next(step for step in finalize["steps"] if step.get("name") == "Emit repository dispatch continuation")
    assert dispatch_step["env"]["DISPATCH_APP_TOKEN"] == "${{ steps.dispatch_app_token.outputs.token }}"
    assert "${{ github.token }}" not in str(dispatch_step)
    assert "PERSONAL_ACCESS_TOKEN" not in text


    payload_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_payload")
    guard_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_guard")
    token_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_app_token")
    assert finalize["steps"].index(payload_step) < finalize["steps"].index(guard_step) < finalize["steps"].index(token_step)
    assert "codex-review loop guard-dispatch" in guard_step["run"]
    assert "dispatch-ledger.json" in guard_step["run"]
    assert "cp -R codex-review-artifacts/final-state/. codex-review-artifacts/dispatch/" in guard_step["run"]
    assert "codex-review loop append-dispatch-ledger" in guard_step["run"]
    assert "dispatch_state_artifact_name=\"codex-loop-dispatch-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}\"" in guard_step["run"]
    assert "'.state_artifact_name = $state_artifact_name'" in guard_step["run"]
    assert "dispatch_duplicate" in reusable_text()


def test_continuation_dispatch_payload_matches_dispatch_adapter_contract():
    finalize = reusable_jobs()["finalize-stage"]
    payload_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_payload")
    run = payload_step["run"]
    env = payload_step["env"]
    assert env["FINAL_NEXT_STAGE"] == "${{ steps.finalize.outputs.next_stage }}"
    assert env["FINAL_STATE_RUN_ID"] == "${{ steps.finalize.outputs.state_run_id }}"
    assert env["FINAL_STATE_ARTIFACT_NAME"] == "${{ steps.finalize.outputs.state_artifact_name }}"
    assert "next_iteration=$(( INPUT_ITERATION + 1 ))" in run
    assert "--arg schema_version \"codex-loop-dispatch-payload.v2\"" in run
    for key in (
        "schema_version", "next_stage", "stage", "head_sha", "base_ref", "iteration",
        "correlation_id", "state_run_id", "state_artifact_name", "requested_by",
        "dry_run", "max_iterations", "pr_number",
    ):
        assert f"{key}:" in run
    dispatch = DISPATCH.read_text(encoding="utf-8")
    assert "PAYLOAD_STAGE: ${{ github.event.client_payload.stage }}" in dispatch


def test_continuation_dispatch_is_guarded_off_in_dry_run_and_terminal_paths():
    finalize = reusable_jobs()["finalize-stage"]
    initial_guard = "${{ steps.finalize.outputs.should_redispatch == 'true' && inputs.dry_run == false && needs.trust-and-stale-guard.outputs.trusted == 'true' && needs.trust-and-stale-guard.outputs.fork == 'false' }}"
    live_guard = "${{ steps.finalize.outputs.should_redispatch == 'true' && inputs.dry_run == false && needs.trust-and-stale-guard.outputs.trusted == 'true' && needs.trust-and-stale-guard.outputs.fork == 'false' && steps.dispatch_guard.outputs.guard_ok == 'true' }}"
    initially_guarded = {
        "Checkout trusted Codex loop core for dispatch",
        "Install trusted codex-review helper for dispatch",
        "Build continuation dispatch context and payload",
        "Download final state bundle for dispatch guard",
        "Guard continuation dispatch ledger",
    }
    live_guarded = {
        "Mint GitHub App installation token for dispatch",
        "Emit repository dispatch continuation",
        "Upload dispatch continuation artifact",
    }
    for step in finalize["steps"]:
        name = step.get("name")
        if name in initially_guarded:
            assert step.get("if") == initial_guard, name
        if name in live_guarded:
            assert step.get("if") == live_guard, name


def test_dispatch_guard_failure_is_normalized_to_canonical_workflow_outputs():
    finalize = reusable_jobs()["finalize-stage"]
    guard_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_guard")
    effective_step = next(step for step in finalize["steps"] if step.get("id") == "effective_outputs")
    token_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_app_token")
    assert finalize["steps"].index(guard_step) < finalize["steps"].index(effective_step) < finalize["steps"].index(token_step)
    assert 'echo "guard_ok=${guard_ok}"' in guard_step["run"]
    assert 'echo "guard_terminal_reason=${guard_terminal_reason}"' in guard_step["run"]
    assert 'if [[ "${guard_ok}" != "true" ]]; then' in guard_step["run"]
    assert "exit 0" in guard_step["run"]
    run = effective_step["run"]
    assert effective_step["if"] == "${{ always() }}"
    assert effective_step["env"]["GUARD_OK"] == "${{ steps.dispatch_guard.outputs.guard_ok }}"
    assert effective_step["env"]["GUARD_REASON"] == "${{ steps.dispatch_guard.outputs.guard_terminal_reason }}"
    assert 'if [[ "${GUARD_OK:-}" == "false" && -n "${GUARD_REASON:-}" ]]; then' in run
    assert 'next_stage=""' in run
    assert 'lgtm="false"' in run
    assert 'should_redispatch="false"' in run
    assert 'terminal_reason="${GUARD_REASON}"' in run
    for reason in ("dispatch_duplicate", "max_iterations", "oscillation_detected"):
        assert reason in run

def test_fix_continuation_payload_uses_verified_updated_head_sha():
    finalize = reusable_jobs()["finalize-stage"]
    payload_step = next(step for step in finalize["steps"] if step.get("id") == "dispatch_payload")
    run = payload_step["run"]
    assert 'dispatch_head_sha="${INPUT_HEAD_SHA}"' in run
    assert 'if [[ "${INPUT_STAGE}" == "fix" ]]; then' in run
    assert '[[ -z "${FINAL_UPDATED_HEAD_SHA}" ]]' in run
    assert "fix-redispatch-missing-verified-updated-head-sha" in run
    assert 'dispatch_head_sha="${FINAL_UPDATED_HEAD_SHA}"' in run


def test_live_writes_are_gated_behind_dry_run_default():
    reusable = reusable_text()
    dispatch = DISPATCH.read_text(encoding="utf-8")
    manual = MANUAL.read_text(encoding="utf-8")
    assert "default: true" in reusable
    assert 'dry_run="${PAYLOAD_DRY_RUN:-true}"' in dispatch
    assert "default: true" in manual
    assert 'if [[ "${INPUT_DRY_RUN}" == "true" ]]' in reusable


def test_helper_is_installed_from_trusted_core_with_pinned_python():
    text = reusable_text()
    install = "python3 -m pip install --disable-pip-version-check -e trusted-core/setup/codex-review"
    assert text.count(install) == 1 + len(REVIEW_STAGE_JOBS) + len(DESIGN_STAGE_JOBS) + len(FIX_STAGE_JOBS) + len(ISSUE_STAGE_JOBS)
    assert "trusted-core-dispatch/setup/codex-review" in text
    for job_name in (*REVIEW_STAGE_JOBS, *DESIGN_STAGE_JOBS, *FIX_STAGE_JOBS, *ISSUE_STAGE_JOBS):
        assert install in str(reusable_jobs()[job_name])
    assert "pip install --disable-pip-version-check -e pr-head" not in text
    assert "pip install --disable-pip-version-check -e trusted/setup/codex-review" not in text


def test_checkout_topology_uses_trusted_core_target_base_and_pr_head_data():
    job = reusable_jobs()["setup-relay"]
    checkouts = [
        step
        for step in job.get("steps", [])
        if step.get("uses", "").startswith("actions/checkout@")
    ]
    by_path = {step["with"]["path"]: step["with"] for step in checkouts}
    assert set(by_path) == {"trusted-core", "target-base", "pr-head"}
    assert by_path["trusted-core"]["repository"] == "DongwonTTuna-Labs/home-server-infra"
    assert by_path["trusted-core"]["ref"] == "${{ github.workflow_sha }}"
    assert by_path["target-base"]["ref"] == "${{ inputs.base_ref }}"
    assert by_path["pr-head"]["ref"] == "${{ inputs.head_sha }}"
    for with_inputs in by_path.values():
        assert with_inputs["persist-credentials"] is False


def test_pr_head_checkout_is_data_only_not_execution_source():
    text = reusable_text()
    assert "path: pr-head" in text
    assert "path: pr-head-write" in text
    assert "pr-head/setup/codex-review" not in text
    assert "trusted/setup/codex-review" not in text
    assert "path: trusted-core" in text
    assert "PR head data" in text


def test_setup_relay_mints_relay_token_locally_via_oidc():
    text = reusable_text()
    assert "setup-codex-relay" not in text
    job = reusable_jobs()["setup-relay"]
    assert job["permissions"]["id-token"] == "write"
    assert "codex-review oidc relay-token" in str(job)


def test_state_artifact_transport_uses_explicit_run_id():
    text = reusable_text()
    assert "run-id: ${{ inputs.state_run_id }}" in text
    assert "name: ${{ inputs.state_artifact_name }}" in text
    for marker in ("gh run list", "--headSha", "headSha"):
        assert marker not in text


def test_workflow_never_executes_helper_from_pr_head_or_stale_trusted_tree():
    text = all_text()
    assert "pr-head/setup/codex-review" not in text
    assert "trusted/setup/codex-review" not in text
    assert "trusted-core/setup/codex-review" in text


def test_fork_pr_is_blocked_in_trust_guard():
    text = reusable_text()
    assert "head_repo_full_name" in text
    assert 'terminal_reason="fork_pr"' in text
    assert '"${head_repo_full_name}" != "${REPOSITORY}"' in text
    assert "headRefName" in text
    assert "head_ref=${head_ref}" in text


def test_issue_stage_is_handled_in_core_without_issue_write():
    text = all_text()
    assert "issue-stage" in text
    assert "issues: write" not in text
    assert "codex-issue.yml" not in {p.name for p in WORKFLOWS.glob("*.yml")}


def test_stage_jobs_are_explicitly_gated_skeletons():
    jobs = reusable_jobs()
    issue = jobs["issue-stage"]
    assert issue["if"].startswith("${{ always() && inputs.stage ==")
    assert "issue" in issue["if"]
    assert issue["name"] == "Issue Fallback Artifact"
    assert "codex-review issue_fallback infer-reason" in str(issue)
    assert "codex-review issue_fallback plan" in str(issue)
    assert "codex-review issue_fallback compose" in str(issue)
    assert "artifact_missing" in str(issue)
    for job_name in REVIEW_STAGE_JOBS:
        job = jobs[job_name]
        assert job["if"].startswith("${{ always() && inputs.stage == 'review'")
    for job_name in DESIGN_STAGE_JOBS:
        job = jobs[job_name]
        assert "inputs.stage == 'design'" in job["if"]
    for job_name in FIX_STAGE_JOBS:
        job = jobs[job_name]
        assert "inputs.stage == 'fix'" in job["if"]
    assert "inputs.dry_run == false" in jobs["fix-push"]["if"]


def test_trust_guard_enforces_open_state_owner_and_requester():
    text = reusable_text()
    assert '"${state}" != "OPEN"' in text
    assert '"${REPOSITORY_OWNER}" != "DongwonTTuna-Labs"' in text
    assert '"${INPUT_REQUESTED_BY}" != *"[bot]"' in text


def test_finalize_stage_routes_upstream_breakage_to_terminal_reason_not_lgtm():
    text = reusable_text()
    assert '"${VALIDATE_RESULT}" != "success"' in text
    assert '"${TRUST_RESULT}" != "success"' in text
    assert '"${selected_result}" != "success"' in text
    assert 'lgtm="false"' in text
    assert 'terminal_reason="${VALIDATE_REASON:-validation_failed}"' in text
    assert 'terminal_reason="${TRUST_REASON:-stage_failed}"' in text
    assert 'terminal_reason="${SETUP_REASON:-stage_failed}"' in text
    assert 'terminal_reason="${terminal_reason:-stage_failed}"' in text


def test_reusable_and_finalize_expose_normalized_route_outputs():
    workflow = load(REUSABLE)
    workflow_call = (workflow.get("on") or workflow[True])["workflow_call"]
    call_outputs = workflow_call["outputs"]
    finalize = reusable_jobs()["finalize-stage"]
    finalize_outputs = finalize["outputs"]

    assert tuple(call_outputs) == FINALIZE_OUTPUT_KEYS
    assert tuple(finalize_outputs) == FINALIZE_OUTPUT_KEYS
    for key in FINALIZE_OUTPUT_KEYS:
        assert call_outputs[key]["value"] == f"${{{{ jobs.finalize-stage.outputs.{key} }}}}"
        assert finalize_outputs[key] == f"${{{{ steps.effective_outputs.outputs.{key} }}}}"


def test_finalize_shell_writes_all_normalized_outputs_and_state_pointers():
    finalize = reusable_jobs()["finalize-stage"]
    step = next(step for step in finalize["steps"] if step.get("id") == "finalize")
    env = step["env"]
    run = step["run"]

    assert env["CURRENT_RUN_ID"] == "${{ github.run_id }}"
    assert 'state_run_id="${CURRENT_RUN_ID}"' in run
    assert 'updated_head_sha=""' in run
    for key in FINALIZE_OUTPUT_KEYS:
        assert f'echo "{key}=${{{key}}}"' in run
    for stage, artifact in {
        "review": "codex-loop-review-state-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}",
        "design": "codex-loop-design-state-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}",
        "fix": "codex-loop-fix-state-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}",
        "issue": "codex-loop-issue-fallback-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}",
    }.items():
        assert f"{stage})" in run
        assert f'state_artifact_name="{artifact}"' in run
    assert 'state_artifact_name="codex-loop-state-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}.json"' in run


def test_terminal_reason_literals_are_canonical_or_blank():
    schema = load(ROOT / "schemas" / "terminal-reason.v1.json")
    canonical = set(schema["enum"])
    literals = set(re.findall(r'terminal_reason="([^"$]*)"', reusable_text()))
    assert literals
    assert literals <= canonical | {""}
    assert "dry_run" in literals
    assert "dry-run" not in literals


def test_intra_workflow_handoffs_use_job_outputs_not_loose_artifacts():
    jobs = reusable_jobs()
    output_jobs = (
        "validate", "trust-and-stale-guard", "setup-relay",
        "review-collect", "review-combine", "review-techlead",
        "design-prepare", "design-plan", "design-chief",
        "fix-dispatch", "fix-merge-validate", "fix-push",
        *ISSUE_STAGE_JOBS,
    )
    for name in output_jobs:
        assert isinstance(jobs[name].get("outputs"), dict), name
        assert jobs[name]["outputs"], name
    assert "strategy" in jobs["review-axes"]
    assert "strategy" in jobs["design-analyze"]
    assert "strategy" in jobs["fix-run-agents"]
    text = reusable_text()
    upload_names = [
        line.strip()
        for line in text.splitlines()
        if "name:" in line and "loop-state" in line
    ]
    assert upload_names, "loop-state artifact transport must remain"


def test_review_axes_matrix_fans_out_over_all_five_axes():
    jobs = reusable_jobs()
    matrix_axes = jobs["review-axes"]["strategy"]["matrix"]["axis"]
    assert list(matrix_axes) == list(REVIEW_AXES)
    assert jobs["review-axes"]["strategy"]["fail-fast"] is False
    uploads = [
        step
        for step in jobs["review-axes"].get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
    ]
    assert uploads
    assert "${{ matrix.axis }}" in uploads[0]["with"]["name"]


def test_review_pipeline_combines_axes_and_routes_through_techlead():
    text = reusable_text()
    assert "codex-review review combine" in text
    assert "codex-review techlead default-result" in text
    assert "codex-review techlead classify" in text
    assert "codex-review loop route-after-techlead" in text
    assert "route-after-techlead" in text


def test_review_posting_is_disabled_in_dry_run_and_routes_via_artifacts():
    jobs = reusable_jobs()
    text = reusable_text()
    assert "codex-review techlead publish --dry-run" in text
    assert "review-posting-not-disabled" in text
    state_uploads = [
        step
        for step in jobs["review-techlead"].get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
    ]
    assert state_uploads
    assert "codex-loop-review-state" in state_uploads[0]["with"]["name"]
    assert "issues: write" not in text


def test_design_pipeline_runs_inventory_cluster_analyze_plan_chief():
    text = reusable_text()
    assert "codex-review design context" in text
    assert "codex-review design default-inventory" in text
    assert "codex-review design default-clusters" in text
    assert "codex-review design prepare-analysis-matrix" in text
    assert "codex-review design default-analysis" in text
    assert "codex-review design collect-analyses" in text
    assert "codex-review design default-plan" in text
    assert "codex-review design_chief default-result" in text
    assert "codex-review design_chief route" in text
    assert "Run Codex loop stage" not in text


def test_design_analyze_matrix_fans_out_over_prepared_clusters():
    jobs = reusable_jobs()
    analyze = jobs["design-analyze"]
    assert analyze["strategy"]["fail-fast"] is False
    assert analyze["strategy"]["matrix"] == "${{ fromJson(needs.design-prepare.outputs.analysis_matrix) }}"
    assert "needs.design-prepare.outputs.has_analysis_batches == 'true'" in analyze["if"]
    prepare_outputs = jobs["design-prepare"].get("outputs", {})
    assert "analysis_matrix" in prepare_outputs
    assert "has_analysis_batches" in prepare_outputs
    uploads = [
        step
        for step in analyze.get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
    ]
    assert uploads
    assert "${{ matrix.batch_path }}" in uploads[0]["with"]["name"]


def test_design_posting_is_disabled_in_dry_run_and_routes_via_state_bundle():
    jobs = reusable_jobs()
    text = reusable_text()
    assert "codex-review design_chief publish --dry-run" in text
    assert "design-posting-not-disabled" in text
    state_uploads = [
        step
        for step in jobs["design-chief"].get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
    ]
    assert state_uploads
    assert "codex-loop-design-state" in state_uploads[0]["with"]["name"]
    assert "design-state-bundle/design-plan.json" in text
    assert "issues: write" not in text


def test_design_consumes_prior_state_via_explicit_run_id_without_head_sha():
    jobs = reusable_jobs()
    text = reusable_text()
    downloads = [
        step
        for step in jobs["design-prepare"].get("steps", [])
        if str(step.get("uses", "")).startswith("actions/download-artifact@")
    ]
    assert len(downloads) == 1
    with_inputs = downloads[0]["with"]
    assert with_inputs["run-id"] == "${{ inputs.state_run_id }}"
    assert with_inputs["name"] == "${{ inputs.state_artifact_name }}"
    assert "codex-review loop read-state" in str(jobs["design-prepare"])
    for marker in ("gh run list", "--headSha", "headSha"):
        assert marker not in text


def test_fix_pipeline_dispatches_agents_merges_validates_and_bundles_state():
    jobs = reusable_jobs()
    text = reusable_text()
    assert "codex-review fix_dispatch plan" in text
    assert "codex-review fix_dispatch prepare-agents" in text
    assert "Plan fix tasks and prepare run_agents matrix" in text
    assert "codex-review fix_dispatch default-agent-result" in text
    assert "codex-review fix_dispatch validate-agent-result" in text
    assert "codex-review fix_dispatch collect" in text
    assert "codex-review fix_merge premerge" in text
    assert "codex-review fix_merge prepare-merge-model" in text
    assert "codex-review fix_merge default-merged-fix" in text
    assert "codex-review fix_merge validate" in text
    assert "codex-review fix_merge build-semantic-safety-prompt" in text
    assert "codex-review fix_merge validate-semantic-safety" in text
    assert "codex-review push validate-fix --dry-run" not in text
    assert "codex-review push validate-fix" in text
    assert "codex-review push commit-push" in text
    assert "codex-review push check-loop-budget" in text
    assert "codex-loop-fix-state" in text
    assert "fix-state-bundle/merged-fix.json" in text
    assert "fix-state-bundle/validated-fix.json" in text
    assert jobs["fix-dispatch"]["outputs"]["agent_matrix"] == "${{ steps.plan_tasks.outputs.agent_matrix }}"
    assert jobs["fix-dispatch"]["outputs"]["has_agent_tasks"] == "${{ steps.plan_tasks.outputs.has_agent_tasks }}"
    plan_tasks_step = next(step for step in jobs["fix-dispatch"]["steps"] if step.get("id") == "plan_tasks")
    plan_tasks_run = plan_tasks_step["run"]
    assert "agent_matrix=\"$(jq -c . codex-review-artifacts/fix_dispatch/agent-matrix.json)\"" in plan_tasks_run
    assert "has_agent_tasks=\"$(jq -er 'if (.include | length) > 0 then" in plan_tasks_run
    assert "echo \"agent_matrix=${agent_matrix}\"" in plan_tasks_run
    assert "echo \"has_agent_tasks=${has_agent_tasks}\"" in plan_tasks_run
    assert ">> \"${GITHUB_OUTPUT}\"" in plan_tasks_run
    assert jobs["fix-merge-validate"]["outputs"]["next_stage"] == "${{ steps.stage_outputs.outputs.next_stage }}"


def test_issue_stage_is_artifact_only_terminal_fallback():
    jobs = reusable_jobs()
    issue = jobs["issue-stage"]
    text = reusable_text()
    assert issue["permissions"].get("contents") == "read"
    assert issue["permissions"].get("pull-requests") == "read"
    assert issue["permissions"].get("actions") == "read"
    assert issue["permissions"].get("issues") != "write"
    assert "codex-review issue_fallback infer-reason" in text
    assert "codex-review issue_fallback plan" in text
    assert "codex-review issue_fallback compose" in text
    assert "codex-review issue_fallback apply" not in text
    assert "artifact_missing" in text
    assert "codex-loop-issue-fallback-${{ inputs.correlation_id }}-${{ inputs.iteration }}" in text
    assert "fallback_reason=" in text
    assert "fallback_title=" in text
    assert "fallback_body_excerpt=" in text
    assert "issues: write" not in text
    assert "issue-created" not in text
    assert "needs-issue" not in text


def test_fix_run_agents_matrix_fans_out_over_prepared_tasks():
    jobs = reusable_jobs()
    run_agents = jobs["fix-run-agents"]
    assert run_agents["strategy"]["fail-fast"] is False
    assert run_agents["strategy"]["matrix"] == "${{ fromJson(needs.fix-dispatch.outputs.agent_matrix) }}"
    assert "needs.fix-dispatch.outputs.has_agent_tasks == 'true'" in run_agents["if"]
    uploads = [
        step
        for step in run_agents.get("steps", [])
        if str(step.get("uses", "")).startswith("actions/upload-artifact@")
    ]
    assert uploads
    assert "${{ matrix.task_path }}" in uploads[0]["with"]["name"]


def test_fix_consumes_design_state_via_explicit_run_id_without_head_sha():
    jobs = reusable_jobs()
    text = reusable_text()
    downloads = [
        step
        for step in jobs["fix-dispatch"].get("steps", [])
        if str(step.get("uses", "")).startswith("actions/download-artifact@")
    ]
    assert len(downloads) == 1
    with_inputs = downloads[0]["with"]
    assert with_inputs["run-id"] == "${{ inputs.state_run_id }}"
    assert with_inputs["name"] == "${{ inputs.state_artifact_name }}"
    assert "design-state-bundle/design-plan.json" in text
    assert "design-state-bundle/chief-decision.json" in text
    for marker in ("gh run list", "--headSha", "headSha"):
        assert marker not in text


def test_fix_stage_is_two_phase_push_capable_and_read_only_except_app_token():
    jobs = reusable_jobs()
    text = reusable_text()
    for job_name in FIX_STAGE_JOBS:
        perms = jobs[job_name].get("permissions", {})
        assert perms.get("contents") == "read", job_name
        assert perms.get("pull-requests") == "read", job_name
        assert perms.get("id-token") != "write", job_name
    assert "codex-review push validate-fix --dry-run" not in text
    assert "codex-review push validate-fix" in str(jobs["fix-merge-validate"])
    assert "codex-review push commit-push" in str(jobs["fix-push"])
    assert any(step.get("with", {}).get("path") == "pr-head-write" for step in jobs["fix-push"].get("steps", []))
    assert "record-push" not in text
    assert "--arg status \"staged\"" in text
    assert "status:$status" in text
    assert "append-dispatch-ledger" in text
    assert "guard-dispatch" in text
    assert "repos/${REPOSITORY}/dispatches" in str(jobs["finalize-stage"])


def test_fix_push_uses_two_phase_trusted_write_checkout_and_app_token():
    jobs = reusable_jobs()
    text = reusable_text()
    merge_validate = jobs["fix-merge-validate"]
    fix_push = jobs["fix-push"]

    assert "codex-review push validate-fix --dry-run" not in str(merge_validate)
    assert "codex-review push validate-fix" in str(merge_validate)
    assert any(step.get("with", {}).get("path") == "pr-head-write" for step in fix_push.get("steps", []))
    assert "--repo-path pr-head-write" in str(fix_push)
    assert "codex-review auth app-token" in str(fix_push)
    assert "--mode push" in str(fix_push)
    push_envs = [step.get("env", {}) for step in fix_push.get("steps", [])]
    assert any(env.get("GITHUB_TOKEN") == "${{ steps.app_token.outputs.token }}" for env in push_envs)
    assert any("CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON" in env for env in push_envs)
    assert all(env.get("GITHUB_TOKEN") != "${{ github.token }}" for env in push_envs)
    assert "PERSONAL_ACCESS_TOKEN" not in text
    assert "pr-head-write/setup/codex-review" not in text


def test_fix_push_routes_stale_and_no_fix_changes_canonically():
    fix_push = str(reusable_jobs()["fix-push"])
    assert "stale_head|no_fix_changes" in fix_push
    assert 'terminal_reason="${push_status}"' in fix_push
    assert "codex-review push commit-push" in fix_push
    assert "codex-loop-fix-push" in fix_push


def test_finalize_prefers_fix_push_outputs_when_push_job_runs():
    finalize = reusable_jobs()["finalize-stage"]
    step = next(step for step in finalize["steps"] if step.get("id") == "finalize")
    run = step["run"]
    env = step["env"]
    assert "FIX_PUSH_RESULT" in env
    assert 'if [[ "${FIX_PUSH_RESULT}" == "success" ]]; then' in run
    assert 'selected_result="${FIX_PUSH_RESULT}"' in run


def test_design_prepare_verifies_review_state_bundle_before_consuming():
    jobs = reusable_jobs()
    prepare = jobs["design-prepare"]
    steps = prepare["steps"]
    download = next(
        s for s in steps if str(s.get("uses", "")).startswith("actions/download-artifact@")
    )
    assert download.get("continue-on-error") is True
    verify = next(s for s in steps if s.get("id") == "verify_bundle")
    assert "codex-review loop verify-bundle" in verify["run"]
    assert "--kind review" in verify["run"]
    assert "bundle_ok=" in verify["run"]
    assert "bundle_terminal_reason=" in verify["run"]
    assert prepare["outputs"]["bundle_ok"] == "${{ steps.verify_bundle.outputs.bundle_ok }}"
    assert (
        prepare["outputs"]["bundle_terminal_reason"]
        == "${{ steps.verify_bundle.outputs.bundle_terminal_reason }}"
    )
    build = next(s for s in steps if s.get("id") == "prepare")
    assert build["if"] == "${{ steps.verify_bundle.outputs.bundle_ok == 'true' }}"
    assert "needs.design-prepare.outputs.bundle_ok == 'true'" in jobs["design-plan"]["if"]


def test_fix_dispatch_verifies_design_state_bundle_before_consuming():
    jobs = reusable_jobs()
    dispatch = jobs["fix-dispatch"]
    steps = dispatch["steps"]
    download = next(
        s for s in steps if str(s.get("uses", "")).startswith("actions/download-artifact@")
    )
    assert download.get("continue-on-error") is True
    verify = next(s for s in steps if s.get("id") == "verify_bundle")
    assert "codex-review loop verify-bundle" in verify["run"]
    assert "--kind design" in verify["run"]
    assert dispatch["outputs"]["bundle_ok"] == "${{ steps.verify_bundle.outputs.bundle_ok }}"
    assert (
        dispatch["outputs"]["bundle_terminal_reason"]
        == "${{ steps.verify_bundle.outputs.bundle_terminal_reason }}"
    )
    plan = next(s for s in steps if s.get("id") == "plan_tasks")
    assert plan["if"] == "${{ steps.verify_bundle.outputs.bundle_ok == 'true' }}"
    assert "needs.fix-dispatch.outputs.bundle_ok == 'true'" in jobs["fix-merge-validate"]["if"]


def test_finalize_normalizes_missing_state_bundle_to_artifact_missing():
    jobs = reusable_jobs()
    finalize = jobs["finalize-stage"]
    assert "design-prepare" in finalize["needs"]
    assert "fix-dispatch" in finalize["needs"]
    step = next(s for s in finalize["steps"] if s.get("id") == "finalize")
    env = step["env"]
    assert (
        env["DESIGN_PREPARE_BUNDLE_REASON"]
        == "${{ needs.design-prepare.outputs.bundle_terminal_reason }}"
    )
    assert (
        env["FIX_DISPATCH_BUNDLE_REASON"]
        == "${{ needs.fix-dispatch.outputs.bundle_terminal_reason }}"
    )
    run = step["run"]
    assert 'bundle_reason="${DESIGN_PREPARE_BUNDLE_REASON:-}"' in run
    assert 'bundle_reason="${FIX_DISPATCH_BUNDLE_REASON:-}"' in run
    assert 'terminal_reason="${bundle_reason}"' in run
    assert 'terminal_reason="${terminal_reason:-stage_failed}"' in run


def test_fix_push_declares_updated_head_sha_output():
    fix_push = reusable_jobs()["fix-push"]
    assert (
        fix_push["outputs"]["updated_head_sha"]
        == "${{ steps.stage_outputs.outputs.updated_head_sha }}"
    )


def test_fix_push_exports_only_remote_verified_updated_head_sha():
    fix_push = reusable_jobs()["fix-push"]
    normalize = next(
        s for s in fix_push["steps"]
        if s.get("name") == "Normalize push result into stage result"
    )
    norm_run = normalize["run"]
    assert "result_updated_head_sha=" in norm_run
    assert ".updated_head_sha //" in norm_run
    assert 'if has("verified") then .verified else false end' in norm_run
    assert '"${verified}" != "true" || -z "${result_updated_head_sha}"' in norm_run
    assert "pushed-without-verified-updated-head-sha" in norm_run
    assert 'updated_head_sha="${result_updated_head_sha}"' in norm_run
    assert "--arg updated_head_sha" in norm_run

    parse = next(s for s in fix_push["steps"] if s.get("id") == "stage_outputs")
    parse_run = parse["run"]
    assert "updated_head_sha=" in parse_run
    assert ".updated_head_sha //" in parse_run
    assert 'echo "updated_head_sha=${updated_head_sha}"' in parse_run


def test_finalize_exports_verified_updated_head_sha_from_fix_push():
    finalize = reusable_jobs()["finalize-stage"]
    step = next(s for s in finalize["steps"] if s.get("id") == "finalize")
    env = step["env"]
    run = step["run"]
    assert (
        env["FIX_PUSH_UPDATED_HEAD_SHA"]
        == "${{ needs.fix-push.outputs.updated_head_sha }}"
    )
    assert 'state_artifact_name="${FIX_PUSH_UPDATED_HEAD_SHA:-}"' not in run
    assert 'state_artifact_name="codex-loop-fix-push-${INPUT_CORRELATION_ID}-${INPUT_ITERATION}"' in run
    assert 'updated_head_sha="${FIX_PUSH_UPDATED_HEAD_SHA:-}"' in run
    assert 'echo "updated_head_sha=${updated_head_sha}"' in run


TRUSTED = "needs.trust-and-stale-guard.outputs.trusted == 'true'"
NON_FORK = "needs.trust-and-stale-guard.outputs.fork == 'false'"
TRUSTED_ENTRYPOINT_STAGE_JOBS = (
    "review-collect",
    "design-prepare",
    "fix-dispatch",
    "fix-push",
    "issue-stage",
)
DISPATCH_SECRET_OR_WRITE_STEPS = {
    "Mint GitHub App installation token for dispatch",
    "Emit repository dispatch continuation",
}


def test_fix_push_requires_trusted_non_fork_same_repo():
    jobs = reusable_jobs()
    fix_push = jobs["fix-push"]
    cond = fix_push["if"]
    assert TRUSTED in cond
    assert NON_FORK in cond
    assert "inputs.dry_run == false" in cond
    assert "needs.fix-merge-validate.result == 'success'" in cond
    assert "trust-and-stale-guard" in fix_push["needs"]


def test_dispatch_secret_and_write_steps_require_trusted_and_non_fork():
    finalize = reusable_jobs()["finalize-stage"]
    dispatch_steps = [
        step
        for step in finalize["steps"]
        if "should_redispatch == 'true'" in str(step.get("if", ""))
    ]
    assert len(dispatch_steps) == 8
    for step in dispatch_steps:
        cond = step["if"]
        assert TRUSTED in cond, step.get("name")
        assert NON_FORK in cond, step.get("name")
    secret_steps = {step.get("name") for step in dispatch_steps} & DISPATCH_SECRET_OR_WRITE_STEPS
    assert secret_steps == DISPATCH_SECRET_OR_WRITE_STEPS


def test_setup_relay_is_gated_on_trusted_before_oidc_and_secrets():
    relay = reusable_jobs()["setup-relay"]
    cond = relay["if"]
    assert TRUSTED in cond
    assert "needs.validate.outputs.valid == 'true'" in cond
    assert "trust-and-stale-guard" in relay["needs"]
    assert relay["permissions"]["id-token"] == "write"
    assert "codex-review oidc relay-token" in str(relay)


def test_trust_guard_classifies_fork_untrusted_owner_and_requester_from_live_pr_state():
    guard = reusable_jobs()["trust-and-stale-guard"]
    run = next(step for step in guard["steps"] if step.get("id") == "guard")["run"]
    assert "gh pr view" in run
    for field in ("state", "headRefOid", "baseRefName", "headRepository", "author"):
        assert field in run, field
    assert 'author_login="$(jq -r' in run
    assert '"${head_repo_full_name}" != "${REPOSITORY}"' in run
    assert 'terminal_reason="fork_pr"' in run
    assert 'terminal_reason="untrusted_repository_owner"' in run
    assert 'terminal_reason="untrusted_requester"' in run
    assert '"${INPUT_REQUESTED_BY}" != "${author_login}"' in run


def test_stage_entrypoints_directly_require_trusted_guard():
    jobs = reusable_jobs()
    for name in TRUSTED_ENTRYPOINT_STAGE_JOBS:
        cond = jobs[name]["if"]
        assert TRUSTED in cond, name
        assert "trust-and-stale-guard" in jobs[name]["needs"], name


def test_downstream_stage_jobs_depend_on_trusted_gated_upstream():
    jobs = reusable_jobs()
    gated = set(TRUSTED_ENTRYPOINT_STAGE_JOBS) | {"setup-relay"}
    for name in ALL_STAGE_JOBS:
        if name in TRUSTED_ENTRYPOINT_STAGE_JOBS:
            continue
        needs = set(jobs[name].get("needs", []))
        assert needs & gated, name


def test_untrusted_fork_pr_cannot_reach_relay_secrets_or_dispatch_paths():
    jobs = reusable_jobs()
    assert TRUSTED in jobs["setup-relay"]["if"]
    finalize = reusable_jobs()["finalize-stage"]
    for step in finalize["steps"]:
        name = step.get("name", "")
        if name in DISPATCH_SECRET_OR_WRITE_STEPS:
            assert TRUSTED in step["if"], name
            assert NON_FORK in step["if"], name
    assert TRUSTED in jobs["fix-push"]["if"]
    assert NON_FORK in jobs["fix-push"]["if"]
