# pyright: reportAny=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path


EXPECTED_STAGES = (
    "trusted-controller",
    "review",
    "design",
    "spec-gap",
    "fix",
    "verify",
    "labels",
    "cast",
)

REQUIRED_INPUTS: dict[str, set[str]] = {
    "trusted-controller": {"consumer-workspace", "control-plane-root", "changed-files", "output-path"},
    "review": {"consumer-workspace", "control-plane-root", "output-path"},
    "design": {"consumer-workspace", "repository", "review-input", "spec-root", "spec-path", "output-path", "plan-path"},
    "spec-gap": {"consumer-workspace", "input-path", "comment-path", "status-path"},
    "fix": {"consumer-workspace", "control-plane-root", "spec-sufficiency", "spec-gap-status", "pr-touched", "direct-extra", "changed-files", "output-path", "handoff-path"},
    "verify": {"consumer-workspace", "spec-sufficiency", "spec-gap-status", "fix-status", "output-path"},
    "labels": {"consumer-workspace", "transition", "state-file", "state-output", "status-path", "repository", "pr-number", "remote-apply", "token", "github-api-url"},
    "cast": {
        "consumer-workspace",
        "control-plane-root",
        "repository",
        "pr-number",
        "head-sha",
        "base-ref",
        "consumer-ref",
        "trusted-status",
        "trusted-controller-outcome",
        "trusted-controller-status",
        "trusted-controller-action",
        "model-execution-allowed",
        "write-allowed",
        "commit-allowed",
        "push-allowed",
        "github-mutation-allowed",
        "changed-files",
        "pr-touched",
        "direct-extra",
        "liveness-timeout-minutes",
    },
}

REQUIRED_OUTPUTS: dict[str, set[str]] = {
    "trusted-controller": {"status", "action", "model_execution_allowed", "write_allowed", "commit_allowed", "push_allowed", "github_mutation_allowed", "status_path"},
    "review": {"status", "approval_signal", "read_only", "mutation_allowed", "findings_count", "output_path"},
    "design": {"status", "spec_sufficient", "should_halt", "in_scope_count", "out_of_scope_count", "output_path", "plan_path"},
    "spec-gap": {"status", "should_comment", "should_halt", "comment_path", "status_path"},
    "fix": {"status", "scope_ok", "noop", "should_commit", "should_push", "output_path", "handoff_path"},
    "verify": {"approved", "output_path", "jq_predicate"},
    "labels": {"changed", "operation_count", "status_path", "state_output", "github_pr_label_mutation_attempted", "remote_apply_status"},
    "cast": {"status", "decision", "terminal", "conclusion", "summary", "final_status_path"},
}

HELPERS = {
    "trusted-controller": "trusted_controller.py",
    "review": "review.py",
    "design": "design.py",
    "spec-gap": "spec_gap.py",
    "fix": "fix.py",
    "verify": "verify.py",
    "labels": "labels.py",
    "cast": "cast_driver.py",
}


class ContractError(AssertionError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ContractError(f"missing action file: {path}") from exc


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def section_body(text: str, key: str, indent: int = 0) -> str:
    lines = text.splitlines()
    prefix = " " * indent
    for index, line in enumerate(lines):
        if line.startswith(prefix) and indent_of(line) == indent and re.match(rf"^{prefix}{re.escape(key)}\s*:", line):
            body: list[str] = []
            for child in lines[index + 1 :]:
                if child.strip() and indent_of(child) <= indent:
                    break
                body.append(child)
            return "\n".join(body)
    return ""


def section_keys(text: str, section: str) -> set[str]:
    body = section_body(text, section)
    return set(re.findall(r"(?m)^  ([A-Za-z0-9_-]+)\s*:", body))


def run_line_indices(text: str) -> list[int]:
    return [index for index, line in enumerate(text.splitlines()) if re.match(r"^\s*run\s*:\s*\|", line)]


def action_step_block(text: str, step_id: str) -> str:
    lines = text.splitlines()
    id_pattern = re.compile(rf"^\s*-\s*id\s*:\s*{re.escape(step_id)}\s*$")
    for index, line in enumerate(lines):
        if not id_pattern.match(line):
            continue
        base_indent = indent_of(line)
        body = [line]
        for child in lines[index + 1 :]:
            if re.match(rf"^ {{{base_indent}}}-\s+", child):
                break
            body.append(child)
        return "\n".join(body)
    return ""


def assert_cast_spec_gap_comment_upsert_contract(text: str) -> None:
    step = action_step_block(text, "upsert-spec-gap-comment")
    require(bool(step), "cast action must define a spec-gap comment upsert step")
    for snippet in (
        "steps.decide.outputs.decision == 'spec-gap-halt'",
        "steps.spec-gap.outputs.should_comment == 'true'",
        "steps.spec-gap.outputs.comment_path != ''",
        "inputs.github-mutation-allowed == 'true'",
        "env.GRIMOIRE_GITHUB_PAT != ''",
        "upsert-spec-gap-comment",
        "--comment-path \"${{ steps.spec-gap.outputs.comment_path }}\"",
        "--github-mutation-allowed \"${{ inputs.github-mutation-allowed }}\"",
        "--github-output \"$GITHUB_OUTPUT\"",
    ):
        require(snippet in step, f"cast spec-gap comment upsert missing required guarded snippet: {snippet}")
    for forbidden in ("steps.complete.outputs", "conclusion == 'failure'", "label_transition == 'fizzled'", "gh pr comment", "gh issue comment", "GITHUB_TOKEN", "github.token"):
        require(forbidden not in step, f"cast spec-gap comment upsert contains forbidden failure-path or alternate-stack snippet: {forbidden}")


def assert_run_blocks_are_bash(text: str, stage: str) -> None:
    lines = text.splitlines()
    for run_index in run_line_indices(text):
        shell_found = False
        scan = run_index - 1
        while scan >= 0:
            line = lines[scan]
            if re.match(r"^\s*-\s+", line):
                break
            if re.match(r"^\s*shell\s*:\s*bash\s*$", line):
                shell_found = True
                break
            if re.match(r"^\s*shell\s*:", line):
                raise ContractError(f"{stage} has a non-bash run block shell")
            scan -= 1
        require(shell_found, f"{stage} run block missing shell: bash")

        base_indent = indent_of(lines[run_index])
        body: list[str] = []
        for child in lines[run_index + 1 :]:
            if child.strip() and indent_of(child) <= base_indent:
                break
            body.append(child)
        body_text = "\n".join(body)
        require(not re.search(r"\b(?:python3?|node|ruby)\s+<<", body_text), f"{stage} embeds an interpreter heredoc in action.yml")
        require(not re.search(r"(?m)^\s*cat\s+<<", body_text), f"{stage} embeds generated scripts in action.yml")


def assert_helper_contract(actions_root: Path, stage: str, text: str) -> None:
    scripts_dir = actions_root / stage / "scripts"
    helper = scripts_dir / HELPERS[stage]
    require(scripts_dir.is_dir(), f"{stage} missing scripts directory")
    require(helper.is_file(), f"{stage} missing helper script: scripts/{HELPERS[stage]}")
    require(os.access(helper, os.X_OK), f"{stage} helper is not executable: {helper}")
    require(f"$GITHUB_ACTION_PATH/scripts/{HELPERS[stage]}" in text, f"{stage} action must call its stage-local helper")
    helper_refs = set(re.findall(r"\$GITHUB_ACTION_PATH/scripts/([A-Za-z0-9_.-]+)", text))
    require(helper_refs == {HELPERS[stage]}, f"{stage} helper references drifted: {sorted(helper_refs)}")
    forbidden_paths = (".github/scripts", "scripts/grimoire")
    for marker in forbidden_paths:
        require(marker not in text, f"{stage} helper path escapes action-local scripts: {marker}")


def assert_action(actions_root: Path, stage: str) -> None:
    action_path = actions_root / stage / "action.yml"
    text = read_text(action_path)
    require(re.search(r"(?m)^runs\s*:\s*$\n^  using\s*:\s*composite\s*$", text) is not None, f"{stage} must use composite runs")
    input_keys = section_keys(text, "inputs")
    output_keys = section_keys(text, "outputs")
    missing_inputs = REQUIRED_INPUTS[stage] - input_keys
    missing_outputs = REQUIRED_OUTPUTS[stage] - output_keys
    require(not missing_inputs, f"{stage} missing required inputs: {', '.join(sorted(missing_inputs))}")
    require(not missing_outputs, f"{stage} missing required outputs: {', '.join(sorted(missing_outputs))}")
    assert_run_blocks_are_bash(text, stage)
    assert_helper_contract(actions_root, stage, text)
    if stage == "cast":
        assert_cast_spec_gap_comment_upsert_contract(text)


def assert_actions_contract(actions_root: Path) -> None:
    require(actions_root.is_dir(), f"actions root does not exist: {actions_root}")
    discovered = tuple(sorted(path.name for path in actions_root.iterdir() if (path / "action.yml").is_file()))
    require(discovered == tuple(sorted(EXPECTED_STAGES)), f"stage set drifted: {list(discovered)}")
    for stage in EXPECTED_STAGES:
        assert_action(actions_root, stage)


def test_grimoire_action_contract() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    assert_actions_contract(repo_root / "actions" / "grimoire")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate Grimoire composite action contracts.")
    parser.add_argument("--actions", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        assert_actions_contract(args.actions.resolve())
    except ContractError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print("action contract ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
