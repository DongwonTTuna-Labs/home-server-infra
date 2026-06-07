"""Contract for the single-run sequential Codex loop core.

Private-repo infra: no OIDC, no GitHub App, no trust/security gates, no loop cap.
The core is four jobs — validate -> setup-state -> run-stage -> finalize. A SINGLE
run executes review -> design -> fix -> push SEQUENTIALLY inside the one run-stage
job (skipping stages that aren't needed); repository_dispatch is used ONLY to start
the NEXT loop iteration after a remote-verified push. It runs live via a static
relay key and a permanent PAT, looping until LGTM.
"""
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
CORE = ROOT / ".github" / "workflows" / "codex-loop-reusable.yml"
RELAY_ENDPOINT = "https://relay-ai.dongwontuna.net/v1/responses"
MODEL_ACTION = "openai/codex-action@v1"


def _text() -> str:
    return CORE.read_text(encoding="utf-8")


def _doc() -> dict:
    return yaml.safe_load(_text())


def _run_stage_steps() -> list:
    return _doc()["jobs"]["run-stage"]["steps"]


def _step_by_id(steps: list, step_id: str) -> dict:
    for step in steps:
        if step.get("id") == step_id:
            return step
    raise AssertionError(f"step id not found: {step_id}")


def test_four_job_pipeline_in_order():
    jobs = list(_doc()["jobs"].keys())
    assert jobs == ["validate", "setup-state", "run-stage", "finalize"], jobs


def test_no_stage_input_remains():
    on = _doc().get("on", _doc().get(True))
    call_inputs = on["workflow_call"]["inputs"]
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
    # Each completion step exposes the next stage as a step output.
    review_complete = _step_by_id(steps, "review_complete")
    assert 'echo "next_stage=' in review_complete["run"]
    design_complete = _step_by_id(steps, "design_complete")
    assert 'echo "next_stage=' in design_complete["run"]
    fix_complete = _step_by_id(steps, "fix_complete")
    assert 'echo "next_stage=' in fix_complete["run"]

    # Review steps are the always-on first stage: they never gate on a prior
    # stage's next_stage output.
    review_complete_if = review_complete.get("if", "")
    assert "next_stage" not in review_complete_if, review_complete_if

    # Design steps gate on the review completion's next_stage == design.
    design_gate = "steps.review_complete.outputs.next_stage == 'design'"
    design_ids = ("design_inventory_prepare", "design_clusters_prepare",
                  "design_plan_prepare", "design_chief_prepare",
                  "design_complete")
    for step_id in design_ids:
        cond = _step_by_id(steps, step_id).get("if", "")
        assert design_gate in cond, (step_id, cond)

    # Fix steps gate on the design completion's next_stage == fix.
    fix_gate = "steps.design_complete.outputs.next_stage == 'fix'"
    fix_ids = ("fix_dispatch_prepare", "fix_merge_prepare",
               "fix_semantic_prepare", "fix_complete")
    for step_id in fix_ids:
        cond = _step_by_id(steps, step_id).get("if", "")
        assert fix_gate in cond, (step_id, cond)

    # Push steps gate on the fix completion's next_stage == push.
    push_gate = "steps.fix_complete.outputs.next_stage == 'push'"
    for step_id in ("push_commit", "push_complete"):
        cond = _step_by_id(steps, step_id).get("if", "")
        assert push_gate in cond, (step_id, cond)


def test_design_fix_push_read_in_run_bundles_not_prior_state():
    steps = _run_stage_steps()
    # Design reads the review bundle's techlead-decision.
    design_prep = _step_by_id(steps, "design_inventory_prepare")["run"]
    assert "review-state-bundle/techlead-decision.json" in design_prep
    # Fix reads the design bundle's plan + chief decision.
    fix_prep = _step_by_id(steps, "fix_dispatch_prepare")["run"]
    assert "design-state-bundle/design-plan.json" in fix_prep
    assert "design-state-bundle/chief-decision.json" in fix_prep
    # Push reads the fix bundle's merged/validated fix.
    push = _step_by_id(steps, "push_commit")["run"]
    assert "fix-state-bundle/merged-fix.json" in push
    assert "fix-state-bundle/validated-fix.json" in push


def test_no_workflow_call_secrets_and_no_secret_refs():
    # Credentials come from runner env vars, not GitHub secrets.
    on = _doc().get("on", _doc().get(True))
    assert "secrets" not in on["workflow_call"], "core must declare no workflow_call secrets"
    assert "${{ secrets." not in _text(), "no secrets.* references allowed"


def test_relay_key_read_from_runner_env_and_fed_to_model():
    text = _text()
    # A capture step reads the runner env var (fail-fast) and exposes it.
    assert "${CODEX_RELAY_API_KEY:?" in text
    assert 'echo "::add-mask::${CODEX_RELAY_API_KEY}"' in text
    seen = 0
    structured = 0
    fix_agents_seen = False
    homes = []
    for job in _doc()["jobs"].values():
        for step in job.get("steps", []) or []:
            if step.get("uses") == MODEL_ACTION:
                seen += 1
                w = step.get("with") or {}
                assert w.get("openai-api-key") == "${{ steps.relay_key.outputs.key }}"
                assert w.get("responses-api-endpoint") == RELAY_ENDPOINT
                # gpt-5.5 defaults to medium effort; the loop runs every model
                # step at the highest reasoning tier.
                assert w.get("effort") == "xhigh", step.get("name")
                # Container is the isolation boundary; the action's sudo-drop
                # sandbox needs passwordless sudo the runner doesn't grant.
                assert w.get("safety-strategy") == "unsafe"
                # Runner container can't nest user namespaces, so codex must not
                # use bubblewrap — run commands directly (full access).
                assert w.get("sandbox") == "danger-full-access", step.get("name")
                assert w.get("working-directory") == "pr-head", step.get("name")
                # Fresh per-step Codex home (persistent runner would otherwise
                # accumulate duplicate keys in ~/.codex/config.toml).
                ch = w.get("codex-home", "")
                assert ch.startswith("${{ runner.temp }}/codex-home/"), step.get("name")
                homes.append(ch)
                name = step.get("name", "")
                if not name.startswith("Run live Codex"):
                    continue
                if "fix agents" in name:
                    # Multi-file emitter writes agents/*/result.json itself.
                    fix_agents_seen = True
                else:
                    # Single-JSON steps capture deterministic output via the
                    # codex-action structured-output contract.
                    assert w.get("output-file"), f"missing output-file: {name}"
                    assert w.get("output-schema-file"), (
                        f"missing output-schema-file: {name}"
                    )
                    structured += 1
    assert seen >= 6, f"expected >=6 model steps, saw {seen}"
    assert structured >= 5, f"expected >=5 structured-output steps, saw {structured}"
    assert fix_agents_seen, "fix agents step not found"
    # Per-step codex-home must be unique so invocations never share config.toml.
    assert len(homes) == len(set(homes)) == seen, homes
    # Each structured step emits its OpenAI strict schema before running.
    assert text.count("schema openai-strict") >= 5


def test_push_and_dispatch_use_the_pat_from_runner_env():
    text = _text()
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
    setup = _doc()["jobs"]["setup-state"]["steps"]
    boot = _step_by_id(setup, "state")["run"]
    assert "codex-review loop read-state --out" in boot
    assert "--loop-state" not in boot


def test_continuation_dispatch_payload_has_no_stage_or_state():
    text = _text()
    resolve = _step_by_id(_doc()["jobs"]["finalize"]["steps"], "resolve")["run"]
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
    dispatch_ifs = [
        step.get("if", "")
        for step in _doc()["jobs"]["finalize"]["steps"]
        if "dispatches" in str(step.get("run", "")) or "repository_dispatch" in str(step.get("name", "")).lower()
    ]
    assert dispatch_ifs, "expected a continuation dispatch step"
    for cond in dispatch_ifs:
        assert cond == "${{ steps.resolve.outputs.dispatch_candidate == 'true' }}", cond
