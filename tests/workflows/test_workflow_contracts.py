from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_DIR = REPO_ROOT / ".github" / "workflows"
EXPECTED_WORKFLOWS = {
    "codex-loop-reusable.yml": {"workflow_call"},
    "codex-loop-dispatch.yml": {"repository_dispatch"},
    "codex-loop-manual.yml": {"workflow_dispatch"},
}
REQUIRED_EVENT_TYPE_MARKERS = {
    "codex-loop-dispatch.yml": "codex-loop",
}
REUSABLE_INPUTS = (
    "pr_number",
    "head_sha",
    "base_ref",
    "stage",
    "iteration",
    "correlation_id",
    "requested_by",
    "max_iterations",
    "dry_run",
)
DISPATCH_REQUIRED_PAYLOAD_FIELDS = (
    "pr_number",
    "head_sha",
    "base_ref",
    "stage",
    "iteration",
    "correlation_id",
    "requested_by",
)
TRUST_AND_STALE_MARKERS = (
    "stale-head-sha",
    "fork-pr",
    "untrusted-requester",
)
REUSABLE_PERMISSION_CEILING = {
    "contents": "write",
    "pull-requests": "write",
    "id-token": "write",
}

FORBIDDEN_PERMISSION_PATTERNS = (
    re.compile(r"(?im)^\s*permissions\s*:\s*write-all\s*(?:#.*)?$"),
    re.compile(r"(?im)^\s*permissions\s*:\s*\{[^}\n]*write-all[^}\n]*}\s*(?:#.*)?$"),
)
FORBIDDEN_ORCHESTRATION_PATTERNS = (
    re.compile(r"\bgh\s+pr\s+edit\b[^\n]*--add-label\b"),
    re.compile(r"\bgh\s+issue\s+edit\b[^\n]*--add-label\b"),
    re.compile(r"\bactions-ecosystem/action-add-labels\b"),
    re.compile(r"\bgh\s+pr\s+comment\b"),
    re.compile(r"\bgh\s+issue\s+comment\b"),
)
SCHEMA_GUARD_MARKERS = (
    "payload schema",
    "payload_schema",
    "validate-payload",
    "validate_payload",
    "jsonschema",
    "jq -e",
    "fromJSON",
    "fromJson",
    "github.event.inputs.payload",
    "github.event.client_payload",
)


def read_workflow(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def assert_no_write_all_permissions(workflow_text: str) -> None:
    for pattern in FORBIDDEN_PERMISSION_PATTERNS:
        assert not pattern.search(workflow_text), "workflow permissions must not use write-all"


def assert_no_label_or_comment_orchestration(workflow_text: str) -> None:
    for pattern in FORBIDDEN_ORCHESTRATION_PATTERNS:
        assert not pattern.search(workflow_text), (
            "workflow must not mutate labels or comments for orchestration state"
        )


def assert_expected_triggers(workflow_text: str, expected_triggers: set[str]) -> None:
    for trigger in expected_triggers:
        assert re.search(rf"(?m)^\s*{re.escape(trigger)}\s*:", workflow_text), (
            f"workflow must declare the {trigger} trigger"
        )


def assert_expected_event_type_marker(workflow_text: str, event_type: str) -> None:
    assert event_type in workflow_text, (
        f"workflow must include the {event_type} repository_dispatch event type marker"
    )


def assert_payload_schema_guard(workflow_text: str) -> None:
    assert any(marker in workflow_text for marker in SCHEMA_GUARD_MARKERS), (
        "workflow must include an explicit payload schema/validation guard"
    )


def assert_minimal_permission_block(workflow_text: str) -> None:
    assert_no_write_all_permissions(workflow_text)
    if re.search(r"(?m)^permissions:\s*\{\}\s*(?:#.*)?$", workflow_text):
        return

    permissions_blocks = re.findall(
        r"(?ms)^permissions:\s*\n((?:^[ ]{2,}[A-Za-z0-9_-]+:\s*(?:read|write|none)\s*(?:#.*)?\n?)+)",
        workflow_text,
    )
    assert permissions_blocks, "workflow must declare an explicit permissions block"

    for block in permissions_blocks:
        entries = re.findall(r"(?m)^\s+([A-Za-z0-9_-]+):\s*(read|write|none)\s*(?:#.*)?$", block)
        assert entries, "workflow permissions must be enumerated by scope"
        for scope, level in entries:
            if level == "write":
                assert scope in {"actions", "checks", "contents", "id-token", "pull-requests"}, (
                    f"workflow write permission for {scope} is not allowlisted"
                )


def assert_required_markers(workflow_text: str, markers: tuple[str, ...], message: str) -> None:
    missing = [marker for marker in markers if marker not in workflow_text]
    assert not missing, f"{message}: missing {', '.join(missing)}"


def assert_concurrency_contract(workflow_text: str, expected_group_fragment: str) -> None:
    assert re.search(r"(?m)^concurrency\s*:", workflow_text), "workflow must declare concurrency"
    assert expected_group_fragment in workflow_text, "workflow concurrency group must bind loop identity"
    assert re.search(r"(?m)^\s*cancel-in-progress\s*:\s*(?:true|false)\s*$", workflow_text), (
        "workflow concurrency must declare cancel-in-progress"
    )


def workflow_call_input_block(workflow_text: str, input_name: str) -> str:
    match = re.search(rf"(?m)^      {re.escape(input_name)}:\s*$", workflow_text)
    assert match, f"workflow_call input {input_name} must be declared"
    following_text = workflow_text[match.end() :]
    next_input = re.search(r"(?m)^      [A-Za-z0-9_-]+:\s*$", following_text)
    return following_text[: next_input.start()] if next_input else following_text


def workflow_dispatch_input_block(workflow_text: str, input_name: str) -> str:
    match = re.search(rf"(?m)^      {re.escape(input_name)}:\s*$", workflow_text)
    assert match, f"manual workflow_dispatch input {input_name} must be declared"
    following_text = workflow_text[match.end() :]
    next_input = re.search(r"(?m)^      [A-Za-z0-9_-]+:\s*$", following_text)
    return following_text[: next_input.start()] if next_input else following_text


def assert_reusable_inputs(workflow_text: str) -> None:
    input_blocks = {input_name: workflow_call_input_block(workflow_text, input_name) for input_name in REUSABLE_INPUTS}
    for input_name in ("pr_number", "iteration", "max_iterations"):
        assert "type: number" in input_blocks[input_name], (
            f"workflow_call input {input_name} must stay typed as number"
        )
    assert "type: boolean" in input_blocks["dry_run"], "workflow_call dry_run input must stay boolean"


def assert_max_iteration_guard(workflow_text: str) -> None:
    assert "INPUT_ITERATION > INPUT_MAX_ITERATIONS" in workflow_text, (
        "reusable workflow must reject iterations over max_iterations"
    )
    assert "max-iterations-exceeded" in workflow_text, (
        "reusable workflow must expose max-iterations-exceeded terminal reason"
    )


def assert_trust_and_stale_guards(workflow_text: str) -> None:
    assert_required_markers(
        workflow_text,
        TRUST_AND_STALE_MARKERS,
        "reusable workflow must reject stale, fork, and untrusted actors before relay setup",
    )


def assert_relay_setup_happens_after_validation_and_trust(workflow_text: str) -> None:
    ordered_markers = (
        "Validate payload schema",
        "Trust And Stale Guard",
        "Configure native codex-lb relay",
        "Run live Codex stage",
    )
    positions = [workflow_text.find(marker) for marker in ordered_markers]
    assert all(position >= 0 for position in positions), "reusable workflow must include all relay ordering markers"
    assert positions == sorted(positions), "validation and trust guards must run before relay setup"


def assert_reusable_live_stage_result_contract(workflow_text: str) -> None:
    markers = (
        ".codex-loop-stage-result.json",
        "Write the machine-readable stage decision",
        "Parse stage result JSON",
        "jq -er '.next_stage | strings'",
        "jq -er '.lgtm | booleans | tostring'",
        "jq -er '.should_redispatch | booleans | tostring'",
        "jq -er '.terminal_reason | strings'",
        "invalid-next-stage",
        "terminal-reason-required",
    )
    assert_required_markers(
        workflow_text,
        markers,
        "reusable workflow must parse and validate machine-readable Codex stage result",
    )
    assert "live-stage-placeholder" not in workflow_text, (
        "live workflow outputs must come from the Codex result JSON, not a placeholder"
    )


def assert_reusable_redispatch_disabled_until_fix_push(workflow_text: str) -> None:
    markers = (
        "trusted-fix-push-not-implemented",
        'elif [[ "${should_redispatch}" == "true" ]]; then',
        'should_redispatch="false"',
    )
    assert_required_markers(
        workflow_text,
        markers,
        "reusable workflow must make redispatch terminal until trusted fix-push support exists",
    )
    forbidden_markers = (
        "Dispatch continuation when requested",
        "steps.finalize.outputs.should_redispatch == 'true'",
        "repos/${REPOSITORY}/dispatches",
        "stale-head-sha-before-dispatch",
        '--input - <<< "${dispatch_body}"',
    )
    present = [marker for marker in forbidden_markers if marker in workflow_text]
    assert not present, "reusable workflow must not redispatch without a fix-push marker"


def assert_required_deliverables_not_ignored() -> None:
    required_paths = (
        ".github/workflows/codex-loop-dispatch.yml",
        ".omo/evidence/task-5-actionlint-dispatch.txt",
    )
    result = subprocess.run(
        ["git", "check-ignore", "-v", *required_paths],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode == 1:
        assert result.stdout == ""
        return
    assert result.returncode == 0, result.stderr
    ignored_lines = [line for line in result.stdout.splitlines() if ":!" not in line]
    assert not ignored_lines, "required dispatch deliverables must only match unignore rules"


def assert_dispatch_payload_mapping(workflow_text: str) -> None:
    for field in DISPATCH_REQUIRED_PAYLOAD_FIELDS:
        assert f"PAYLOAD_{field.upper()}: ${{{{ github.event.client_payload.{field} }}}}" in workflow_text, (
            f"dispatch payload field {field} must be read from client_payload"
        )
        assert f"require_non_empty {field} " in workflow_text, (
            f"dispatch payload field {field} must be required before reusable call"
        )
    for field in REUSABLE_INPUTS:
        assert re.search(rf"(?m)^      {field}: ", workflow_text), (
            f"dispatch adapter must map {field} into reusable workflow inputs"
        )
    assert "max_iterations=\"${PAYLOAD_MAX_ITERATIONS:-5}\"" in workflow_text, (
        "dispatch adapter must default max_iterations offline before reusable call"
    )
    assert "dry_run=\"${PAYLOAD_DRY_RUN:-true}\"" in workflow_text, (
        "dispatch adapter must default dry_run to true offline before reusable call"
    )
    assert "dry_run: ${{ needs.validate-payload.outputs.dry_run == 'true' }}" in workflow_text, (
        "dispatch adapter must convert dry_run string output to a boolean workflow_call input"
    )


def assert_manual_input_mapping(workflow_text: str) -> None:
    input_blocks = {field: workflow_dispatch_input_block(workflow_text, field) for field in REUSABLE_INPUTS}
    for field in REUSABLE_INPUTS:
        assert f"{field}: ${{{{ inputs.{field} }}}}" in workflow_text, (
            f"manual adapter must map input {field} directly into reusable workflow"
        )
    assert "default: true" in input_blocks["dry_run"], "manual adapter dry_run must default to true"


def assert_reusable_permission_ceiling(workflow_text: str) -> None:
    assert "uses: ./.github/workflows/codex-loop-reusable.yml" in workflow_text, (
        "adapter must call the reusable core workflow"
    )
    for scope, level in REUSABLE_PERMISSION_CEILING.items():
        assert re.search(rf"(?m)^      {re.escape(scope)}: {level}\s*$", workflow_text), (
            f"adapter caller job must grant reusable permission ceiling {scope}: {level}"
        )


def assert_workflow_contract(path: Path, expected_triggers: set[str]) -> None:
    workflow_text = read_workflow(path)
    assert_expected_triggers(workflow_text, expected_triggers)
    if event_type := REQUIRED_EVENT_TYPE_MARKERS.get(path.name):
        assert_expected_event_type_marker(workflow_text, event_type)
    assert_minimal_permission_block(workflow_text)
    assert_payload_schema_guard(workflow_text)
    assert_no_label_or_comment_orchestration(workflow_text)


def test_dispatch_workflow_contract_requires_codex_loop_event_type_marker() -> None:
    workflow_text = """
name: Codex Loop Dispatch

on:
  repository_dispatch:
    types: [other-event]
"""

    assert_expected_triggers(workflow_text, {"repository_dispatch"})
    with pytest.raises(AssertionError, match="codex-loop"):
        assert_expected_event_type_marker(workflow_text, "codex-loop")


@pytest.mark.parametrize(("filename", "expected_triggers"), EXPECTED_WORKFLOWS.items())
def test_future_codex_loop_workflows_satisfy_contract(filename: str, expected_triggers: set[str]) -> None:
    workflow_path = WORKFLOW_DIR / filename
    if not workflow_path.exists():
        pytest.skip(f"{workflow_path} has not been added yet")

    assert_workflow_contract(workflow_path, expected_triggers)


def test_reusable_workflow_contract_has_guards_concurrency_and_ordering() -> None:
    workflow_text = read_workflow(WORKFLOW_DIR / "codex-loop-reusable.yml")

    assert_reusable_inputs(workflow_text)
    assert_concurrency_contract(workflow_text, "codex-loop-${{ inputs.correlation_id }}")
    assert_max_iteration_guard(workflow_text)
    assert_trust_and_stale_guards(workflow_text)
    assert_relay_setup_happens_after_validation_and_trust(workflow_text)
    assert_reusable_live_stage_result_contract(workflow_text)
    assert_reusable_redispatch_disabled_until_fix_push(workflow_text)


def test_required_dispatch_deliverables_are_not_ignored() -> None:
    assert_required_deliverables_not_ignored()


def test_dispatch_adapter_maps_payload_and_reusable_permission_ceiling() -> None:
    workflow_text = read_workflow(WORKFLOW_DIR / "codex-loop-dispatch.yml")

    assert_concurrency_contract(
        workflow_text,
        "codex-loop-${{ github.event.client_payload.pr_number }}-${{ github.event.client_payload.head_sha }}",
    )
    assert_dispatch_payload_mapping(workflow_text)
    assert_reusable_permission_ceiling(workflow_text)


def test_manual_adapter_maps_inputs_and_reusable_permission_ceiling() -> None:
    workflow_text = read_workflow(WORKFLOW_DIR / "codex-loop-manual.yml")

    assert_manual_input_mapping(workflow_text)
    assert_reusable_permission_ceiling(workflow_text)


@pytest.mark.parametrize(
    "fixture_name",
    [
        "unsafe-write-all.yml",
        "unsafe-inline-write-all.yml",
    ],
)
def test_negative_fixtures_reject_write_all_permissions(fixture_name: str) -> None:
    workflow_text = read_workflow(Path(__file__).parent / "fixtures" / fixture_name)

    with pytest.raises(AssertionError, match="write-all"):
        assert_no_write_all_permissions(workflow_text)


@pytest.mark.parametrize(
    "fixture_name",
    [
        "unsafe-gh-pr-add-label.yml",
        "unsafe-gh-issue-add-label.yml",
        "unsafe-action-add-labels.yml",
    ],
)
def test_negative_fixtures_reject_label_mutation(fixture_name: str) -> None:
    workflow_text = read_workflow(Path(__file__).parent / "fixtures" / fixture_name)

    with pytest.raises(AssertionError, match="labels or comments"):
        assert_no_label_or_comment_orchestration(workflow_text)


def test_negative_sample_rejects_malformed_dispatch_payload_mapping() -> None:
    workflow_text = """
jobs:
  validate-payload:
    steps:
      - env:
          PAYLOAD_PR_NUMBER: ${{ github.event.client_payload.pr_number }}
        run: |
          require_non_empty pr_number "${PAYLOAD_PR_NUMBER}"
  codex-loop:
    uses: ./.github/workflows/codex-loop-reusable.yml
    with:
      pr_number: ${{ fromJSON(needs.validate-payload.outputs.pr_number) }}
"""

    with pytest.raises(AssertionError, match="head_sha"):
        assert_dispatch_payload_mapping(workflow_text)


def test_negative_sample_rejects_unchanged_head_redispatch_without_fix_push_marker() -> None:
    workflow_text = """
jobs:
  finalize:
    steps:
      - name: Dispatch continuation when requested
        if: ${{ inputs.dry_run == false && steps.finalize.outputs.should_redispatch == 'true' }}
        run: |
          current_head_sha="$(jq -r '.headRefOid' <<< "${pr_json}")"
          client_payload="$(jq -nc --arg head_sha "${current_head_sha}" '{head_sha: $head_sha}')"
          gh api "repos/${REPOSITORY}/dispatches" --method POST --input - <<< "${dispatch_body}"
"""

    with pytest.raises(AssertionError, match="redispatch"):
        assert_reusable_redispatch_disabled_until_fix_push(workflow_text)


def test_negative_sample_rejects_missing_stale_sha_marker() -> None:
    workflow_text = """
jobs:
  trust-and-stale-guard:
    steps:
      - run: |
          terminal_reason="fork-pr"
          terminal_reason="untrusted-requester"
"""

    with pytest.raises(AssertionError, match="stale-head-sha"):
        assert_trust_and_stale_guards(workflow_text)


def test_negative_sample_rejects_missing_unauthorized_or_fork_actor_marker() -> None:
    workflow_text = """
jobs:
  trust-and-stale-guard:
    steps:
      - run: |
          terminal_reason="stale-head-sha"
"""

    with pytest.raises(AssertionError, match="fork-pr"):
        assert_trust_and_stale_guards(workflow_text)


def test_negative_sample_rejects_missing_max_iteration_guard() -> None:
    workflow_text = """
jobs:
  validate:
    steps:
      - run: |
          echo "iteration=${INPUT_ITERATION}" >> "${GITHUB_OUTPUT}"
"""

    with pytest.raises(AssertionError, match="max_iterations"):
        assert_max_iteration_guard(workflow_text)


def test_negative_sample_rejects_missing_concurrency() -> None:
    workflow_text = """
name: Codex Loop Without Concurrency

on:
  workflow_call:
"""

    with pytest.raises(AssertionError, match="concurrency"):
        assert_concurrency_contract(workflow_text, "codex-loop-${{ inputs.correlation_id }}")
