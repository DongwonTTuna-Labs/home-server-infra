"""Terminal visibility + no-mask contract for the Codex loop reusable core.

Task 25 requires every non-LGTM terminal outcome to be observable without labels,
comments, or issues. The reusable core's ``finalize-stage`` must:

* build a machine-readable ``codex-loop-state.json`` and a human-readable
  ``terminal-summary.md`` and upload them as an artifact for terminal outcomes;
* write the terminal reason, next/manual action, selected state artifact, and an
  artifact lookup hint into ``$GITHUB_STEP_SUMMARY``;
* surface failure terminals as a failed workflow conclusion (no broad
  ``continue-on-error`` / unconditional ``exit 0`` masking) while keeping LGTM,
  dry-run, and expected no-fix outcomes green; and
* source its decision from the canonical ``effective_outputs`` (Task 23), never
  the pre-normalization ``finalize`` step terminal_reason.
"""
import json
import re
from pathlib import Path

from _pipeline import REUSABLE, load, reusable_jobs

ROOT = Path(__file__).resolve().parents[4]

REQUIRED_STATE_KEYS = (
    "schema_version",
    "stage",
    "pr_number",
    "head_sha",
    "base_ref",
    "correlation_id",
    "iteration",
    "terminal_reason",
    "lgtm",
    "should_redispatch",
    "next_stage",
    "state_run_id",
    "state_artifact_name",
    "updated_head_sha",
    "dry_run",
    "selected_result",
)

# Terminal reasons that intentionally keep the workflow conclusion green. The empty
# reason marks an LGTM terminal or an in-progress continuation.
NON_FAILURE_REASONS = {
    "lgtm",
    "dry_run",
    "no_fix_needed",
    "no_fix_changes",
    "empty_patch",
    "issue_created",
}

# Canonical reasons that must NOT be masked as success.
REQUIRED_FAILURE_REASONS = {
    "validation_failed",
    "tests_failed",
    "semantic_safety_missing",
    "semantic_safety_rejected",
    "semantic_safety_hash_mismatch",
    "policy_rejected",
    "stale_head",
    "base_ref_mismatch",
    "pr_closed",
    "fork_pr",
    "untrusted_repository_owner",
    "untrusted_requester",
    "missing_app_credentials",
    "app_token_scope_invalid",
    "push_failed",
    "pushed_unverified",
    "dispatch_failed",
    "dispatch_duplicate",
    "max_iterations",
    "oscillation_detected",
    "artifact_missing",
    "artifact_schema_invalid",
    "model_output_invalid",
    "stage_failed",
}


def _finalize_steps():
    return reusable_jobs()["finalize-stage"]["steps"]


def _step_by_id(step_id):
    return next(s for s in _finalize_steps() if s.get("id") == step_id)


def _non_failure_tokens_from(run_text):
    """Extract the ``""|lgtm|...`` case label used to mark non-failure terminals."""
    match = re.search(r'""\|([a-z_|]+)\)', run_text)
    assert match, "expected a non-failure case label with an empty-reason branch"
    return set(match.group(1).split("|"))


def test_finalize_builds_and_uploads_terminal_state_and_summary_artifact():
    steps = _finalize_steps()
    build = _step_by_id("terminal_state")
    run = build["run"]
    assert "codex-review-artifacts/terminal/codex-loop-state.json" in run
    assert "codex-review-artifacts/terminal/terminal-summary.md" in run

    uploads = [
        s
        for s in steps
        if str(s.get("uses", "")).startswith("actions/upload-artifact@")
        and "codex-loop-terminal" in str((s.get("with") or {}).get("name", ""))
    ]
    assert len(uploads) == 1, "exactly one terminal-state/summary upload expected"
    upload = uploads[0]
    assert (upload["with"]["path"]) == "codex-review-artifacts/terminal"
    assert int(upload["with"]["retention-days"]) <= 14
    assert upload["if"] == "${{ always() }}"


def test_terminal_state_json_includes_required_machine_keys():
    run = _step_by_id("terminal_state")["run"]
    # The jq object that materializes codex-loop-state.json must carry every field
    # the orchestrator and humans need to reconstruct the terminal outcome.
    for key in REQUIRED_STATE_KEYS:
        assert f"{key}:" in run, f"codex-loop-state.json missing key {key}"
    assert '"codex-loop-state.v1"' in run


def test_terminal_summary_carries_human_fields_and_label_free_state_note():
    run = _step_by_id("terminal_state")["run"]
    assert 'cat codex-review-artifacts/terminal/terminal-summary.md >> "${GITHUB_STEP_SUMMARY}"' in run
    assert "Next action" in run
    assert "${manual_action}" in run
    assert "selected state artifact:" in run
    assert "lookup hint: gh run download" in run
    assert "No labels, comments, or issues are used for loop state" in run


def test_terminal_visibility_reads_canonical_effective_outputs_not_finalize():
    for step_id in ("terminal_state", "terminal_conclusion"):
        env = _step_by_id(step_id).get("env", {})
        assert env.get("EFFECTIVE_REASON") == "${{ steps.effective_outputs.outputs.terminal_reason }}", step_id
        # Must not regress to the pre-normalization finalize terminal_reason (Task 23).
        assert "steps.finalize.outputs.terminal_reason" not in json.dumps(env), step_id


def test_terminal_conclusion_fails_only_on_failure_reasons():
    conclusion = _step_by_id("terminal_conclusion")
    run = conclusion["run"]
    assert conclusion["if"] == "${{ always() }}"
    # The wildcard branch (every reason not in the non-failure set) must exit non-zero.
    assert "exit 1" in run
    assert "conclusion=failure" in run
    assert "conclusion=success" in run

    non_failure = _non_failure_tokens_from(run)
    assert non_failure == NON_FAILURE_REASONS

    # Every canonical failure reason falls into the wildcard (exit 1) branch.
    assert not (REQUIRED_FAILURE_REASONS & non_failure)


def test_terminal_conclusion_partition_covers_full_canonical_enum():
    schema = load(ROOT / "schemas" / "terminal-reason.v1.json")
    canonical = set(schema["enum"])
    run = _step_by_id("terminal_conclusion")["run"]
    non_failure = _non_failure_tokens_from(run)
    # Non-failure reasons are real canonical reasons, and the failure partition is
    # exactly the remaining canonical reasons (no reason is silently unclassified).
    assert non_failure <= canonical
    failure_partition = canonical - non_failure
    assert REQUIRED_FAILURE_REASONS <= failure_partition


def test_terminal_visibility_steps_do_not_introduce_success_masking():
    for step_id in ("terminal_state", "terminal_conclusion"):
        step = _step_by_id(step_id)
        assert step.get("continue-on-error") is not True, step_id
        assert "exit 0" not in step["run"], step_id
    upload = next(
        s
        for s in _finalize_steps()
        if str(s.get("uses", "")).startswith("actions/upload-artifact@")
        and "codex-loop-terminal" in str((s.get("with") or {}).get("name", ""))
    )
    assert upload.get("continue-on-error") is not True


def test_terminal_visibility_steps_carry_no_secret_or_token_env():
    for step_id in ("terminal_state", "terminal_conclusion"):
        env_text = json.dumps(_step_by_id(step_id).get("env", {}))
        for needle in ("secrets.", "APP_PRIVATE_KEY", "APP_TOKEN", "relay_token", "github.token"):
            assert needle not in env_text, f"{step_id} leaks {needle}"


def test_finalize_exposes_selected_result_for_terminal_state():
    finalize = _step_by_id("finalize")
    assert 'echo "selected_result=${selected_result}"' in finalize["run"]
    assert _step_by_id("terminal_state")["env"].get("SELECTED_RESULT") == "${{ steps.finalize.outputs.selected_result }}"


def test_terminal_artifact_steps_run_after_effective_outputs():
    steps = _finalize_steps()
    order = {s.get("id"): i for i, s in enumerate(steps) if s.get("id")}
    assert order["effective_outputs"] < order["terminal_state"] < order["terminal_conclusion"]
