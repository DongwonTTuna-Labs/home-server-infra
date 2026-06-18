#!/usr/bin/env python3
# pyright: reportAny=false, reportExplicitAny=false, reportUnusedCallResult=false, reportImplicitStringConcatenation=false
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from typing import Any, cast

TEMP_ROOT = pathlib.Path("/var/folders/vz/hx33c759727ftq88cxbgp8r40000gn/T/opencode")
REPO_ROOT = pathlib.Path(__file__).resolve().parents[3]
ACTIONS_ROOT = REPO_ROOT / "actions" / "grimoire"
APPROVE_LENSES = ("f1_oracle", "f2_quality", "f3_real_qa", "f4_scope")
SENTINEL_ENV = {
    "CODEX_RELAY_API_KEY": "GRIMOIRE_FIXTURE_CODEX_RELAY_API_KEY_SENTINEL",
    "CODEX_LOOP_PAT": "GRIMOIRE_FIXTURE_CODEX_LOOP_PAT_SENTINEL",
    "AI_RELAY_API_KEY": "GRIMOIRE_FIXTURE_AI_RELAY_API_KEY_SENTINEL",
    "CF_ACCESS_CLIENT_ID": "GRIMOIRE_FIXTURE_CF_ACCESS_CLIENT_ID_SENTINEL",
    "CF_ACCESS_CLIENT_SECRET": "GRIMOIRE_FIXTURE_CF_ACCESS_CLIENT_SECRET_SENTINEL",
}
TOKEN_PATTERNS = (
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"(?i)\bbearer\s+[A-Za-z0-9._~-]{16,}"),
    re.compile(r"(?i)\b(?:CODEX_RELAY_API_KEY|CODEX_LOOP_PAT|AI_RELAY_API_KEY|CF_ACCESS_CLIENT_ID|CF_ACCESS_CLIENT_SECRET)\s*[:=]\s*[\"']?[^\"'\s]+"),
    re.compile(r"(?i)\b(?:token|secret|password|api[_-]?key)\s*[:=]\s*[\"']?[^\"'\s]+"),
    re.compile(r"(?i)https?://[^\s\"'<>]+(?:token|access_token|api[_-]?key)=[^\s\"'<>]+"),
)
DECIDE_FIXTURE_ROOT = pathlib.Path(__file__).resolve().parent / "decide"
DECIDE_OUTPUT = ".omo/ci/cast-decision.json"
DECIDE_INPUT_PATHS = {
    "preflight": ".omo/ci/cast-preflight.json",
    "review": ".omo/ci/review.json",
    "design": ".omo/ci/design.json",
    "issues": ".omo/ci/issues.json",
    "spec_gap": ".omo/ci/spec-gap.json",
    "fix": ".omo/ci/fix.json",
    "boulder": ".omo/boulder.json",
    "verdict": ".omo/grimoire/verdict.json",
}
DECIDE_EXPECTED_TUPLES: dict[str, tuple[str, str, int, str]] = {
    "spec-gap-advisory": ("spec-gap-halt", "neutral", 0, "spec-needed"),
    "no-actionable-work": ("no-actionable-work", "neutral", 0, "spec-needed"),
    "clear-noop-success": ("clear-noop-terminal", "success", 0, "done"),
    "scoped-push-success": ("scoped-push", "success", 0, "keep-running"),
    "scope-violation-failure": ("fizzled", "failure", 1, "fizzled"),
    "malformed-verdict-failure": ("fizzled", "failure", 1, "fizzled"),
    "preflight-blocked-failure": ("fizzled", "failure", 1, "fizzled"),
}
DECIDE_CASE_DEFINITIONS: tuple[dict[str, Any], ...] = (
    {
        "name": "spec-gap-advisory",
        "label": "spec-gap advisory",
        "preflight": "ok",
        "review": "findings",
        "design": "spec-gap",
        "issues": "ok",
        "spec_gap": "halt",
        "fix": "clear-noop",
        "boulder": "skipped-clear-noop",
        "verdict": "approve",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
    {
        "name": "no-actionable-work",
        "label": "no-actionable-work",
        "preflight": "ok",
        "review": "approved",
        "design": "no-actionable",
        "issues": "ok",
        "spec_gap": "clear",
        "fix": "clear-noop",
        "boulder": "skipped-clear-noop",
        "verdict": "approve",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
    {
        "name": "clear-noop-success",
        "label": "clear-noop success",
        "preflight": "ok",
        "review": "findings",
        "design": "actionable",
        "issues": "ok",
        "spec_gap": "clear",
        "fix": "clear-noop",
        "boulder": "skipped-clear-noop",
        "verdict": "approve",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
    {
        "name": "scoped-push-success",
        "label": "scoped-push success",
        "preflight": "ok",
        "review": "findings",
        "design": "actionable",
        "issues": "ok",
        "spec_gap": "clear",
        "fix": "fixed",
        "boulder": "completed",
        "verdict": "approve",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
    {
        "name": "scope-violation-failure",
        "label": "scope-violation failure",
        "preflight": "ok",
        "review": "findings",
        "design": "actionable",
        "issues": "ok",
        "spec_gap": "clear",
        "fix": "scope-violation",
        "boulder": "completed",
        "verdict": "approve",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
    {
        "name": "malformed-verdict-failure",
        "label": "malformed-verdict failure",
        "preflight": "ok",
        "review": "findings",
        "design": "actionable",
        "issues": "ok",
        "spec_gap": "clear",
        "fix": "fixed",
        "boulder": "completed",
        "verdict": "malformed",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
    {
        "name": "preflight-blocked-failure",
        "label": "preflight-blocked failure",
        "preflight": "blocked",
        "review": "findings",
        "design": "actionable",
        "issues": "ok",
        "spec_gap": "clear",
        "fix": "fixed",
        "boulder": "completed",
        "verdict": "approve",
        "review_outcome": "success",
        "fix_outcome": "success",
        "verify_outcome": "success",
    },
)


class FixtureError(AssertionError):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def require(condition: bool, message: str) -> None:
    if not condition:
        raise FixtureError(message)


def ensure_temp_path(path: pathlib.Path, label: str) -> pathlib.Path:
    resolved = path.resolve()
    temp_root = TEMP_ROOT.resolve()
    try:
        resolved.relative_to(temp_root)
    except ValueError as exc:
        raise FixtureError(f"{label} must be under approved temp root: {temp_root}") from exc
    return resolved


def sanitize(text: object) -> str:
    value = str(text)
    for sentinel in SENTINEL_ENV.values():
        value = value.replace(sentinel, "[REDACTED]")
    for pattern in TOKEN_PATTERNS:
        value = pattern.sub("[REDACTED]", value)
    return value


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> pathlib.Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def read_json(workspace: pathlib.Path, relative: str) -> dict[str, Any]:
    path = workspace / relative
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise FixtureError(f"missing JSON artifact: {relative}") from exc
    except json.JSONDecodeError as exc:
        raise FixtureError(f"malformed JSON artifact {relative}: {exc}") from exc
    require(isinstance(payload, dict), f"JSON artifact must be an object: {relative}")
    return payload


def write_text(path: pathlib.Path, text: str) -> pathlib.Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")
    return path


def expected_decide_tuples() -> dict[str, tuple[str, str, int, str]]:
    return dict(DECIDE_EXPECTED_TUPLES)


def load_decide_fixture_cases() -> list[dict[str, Any]]:
    cases: list[dict[str, Any]] = []
    for definition in DECIDE_CASE_DEFINITIONS:
        name = str(definition["name"])
        cases.append(
            {
                "name": name,
                "label": definition["label"],
                "fixture_dir": str(DECIDE_FIXTURE_ROOT / name),
                "input_paths": dict(DECIDE_INPUT_PATHS),
                "expected": DECIDE_EXPECTED_TUPLES[name],
                "review_outcome": definition["review_outcome"],
                "fix_outcome": definition["fix_outcome"],
                "verify_outcome": definition["verify_outcome"],
            }
        )
    return cases


def decide_preflight_payload(kind: str) -> dict[str, Any]:
    if kind == "blocked":
        return {
            "schema_version": 1,
            "stage": "grimoire-cast",
            "status": "blocked",
            "can_continue": False,
            "blocked_reason": "trusted-controller preflight denied deterministic fixture",
            "does_not_rerun_trusted_controller": True,
        }
    return {
        "schema_version": 1,
        "stage": "grimoire-cast",
        "status": "ok",
        "can_continue": True,
        "trusted_status_path": ".omo/ci/trusted-controller-status.json",
        "trusted_controller_status": "ok",
        "does_not_rerun_trusted_controller": True,
    }


def decide_review_payload(kind: str) -> dict[str, Any]:
    findings = []
    if kind == "findings":
        findings = [finding("Synthetic in-scope Grimoire decide fixture")]
    return {
        "schema_version": 1,
        "stage": "grimoire-review",
        "status": "findings" if findings else "approved",
        "approval_signal": "GRIMOIRE_REVIEW_FINDINGS_PRESENT" if findings else "GRIMOIRE_REVIEW_APPROVED",
        "read_only": True,
        "mutation_allowed": False,
        "findings": findings,
        "findings_count": len(findings),
        "lenses": ["security", "correctness", "maintainability", "repo-policy"],
    }


def decide_design_payload(kind: str) -> dict[str, Any]:
    actionable = {
        "schema_version": 1,
        "stage": "grimoire-design",
        "status": "sufficient",
        "spec_sufficient": True,
        "should_halt": False,
        "halt_reason": "",
        "scope_authority": "OpenSpec and OMO",
        "in_scope": [{"path": "src/lib.rs", "title": "Synthetic in-scope Grimoire decide fixture"}],
        "out_of_scope": [],
        "bindings": [{"target_paths": ["src/lib.rs"], "source": "deterministic fixture"}],
        "target_paths": ["src/lib.rs"],
        "allowed_write_paths": ["src/lib.rs"],
        "missing": [],
        "safety_default_gaps": [],
        "suggested_spec_patch": "",
        "plan_path": ".omo/ci/design-plan.md",
    }
    if kind == "actionable":
        return actionable
    if kind == "no-actionable":
        no_actionable = dict(actionable)
        no_actionable.update(
            {
                "in_scope": [],
                "bindings": [],
                "target_paths": [],
                "allowed_write_paths": [],
            }
        )
        return no_actionable
    if kind == "spec-gap":
        spec_gap = dict(actionable)
        spec_gap.update(
            {
                "status": "insufficient",
                "spec_sufficient": False,
                "should_halt": True,
                "halt_reason": "deterministic spec gap fixture",
                "bindings": [],
                "target_paths": [],
                "allowed_write_paths": [],
                "missing": [{"location": "src/lib.rs:7", "reason": "missing fixture OpenSpec evidence"}],
                "safety_default_gaps": [{"scope": "src/lib.rs:7", "required_default": "halt before fix"}],
                "suggested_spec_patch": "Add fixture OpenSpec evidence before code changes.",
            }
        )
        return spec_gap
    raise FixtureError(f"unsupported decide design fixture kind: {kind}")


def decide_issues_payload(kind: str) -> dict[str, Any]:
    require(kind == "ok", f"unsupported decide issue fixture kind: {kind}")
    return {
        "schema_version": 1,
        "stage": "grimoire-cast",
        "status": "ok",
        "issue_write_only": True,
        "pr_head_write_attempted": False,
        "remote_issue_mutation_attempted": False,
        "design_stage_complete": True,
        "issue_count": 0,
        "created_count": 0,
        "deduped_count": 0,
        "records": [],
    }


def decide_spec_gap_payload(kind: str) -> dict[str, Any]:
    if kind == "halt":
        return {
            "schema_version": 1,
            "stage": "grimoire-spec-gap",
            "status": "halt",
            "should_halt": True,
            "should_comment": True,
            "no_code_or_push_action": True,
            "comment_path": ".omo/ci/spec-gap-comment.md",
        }
    require(kind == "clear", f"unsupported decide spec-gap fixture kind: {kind}")
    return {
        "schema_version": 1,
        "stage": "grimoire-spec-gap",
        "status": "clear",
        "should_halt": False,
        "should_comment": False,
    }


def decide_fix_payload(kind: str) -> dict[str, Any]:
    if kind == "clear-noop":
        return {
            "schema_version": 1,
            "stage": "grimoire-fix",
            "status": "clear-noop",
            "scope_ok": True,
            "noop": True,
            "should_commit": False,
            "should_push": False,
            "changed_files": [],
            "violations": [],
        }
    if kind == "fixed":
        return {
            "schema_version": 1,
            "stage": "grimoire-fix",
            "status": "fixed",
            "scope_ok": True,
            "noop": False,
            "should_commit": True,
            "should_push": True,
            "changed_files": ["src/lib.rs"],
            "allowed_paths": ["src/lib.rs"],
            "allowed_write_paths": ["src/lib.rs"],
            "violations": [],
        }
    if kind == "scope-violation":
        return {
            "schema_version": 1,
            "stage": "grimoire-fix",
            "status": "scope-violation",
            "scope_ok": False,
            "noop": False,
            "should_commit": False,
            "should_push": False,
            "changed_files": ["src/unscoped.rs"],
            "allowed_paths": ["src/lib.rs"],
            "allowed_write_paths": ["src/lib.rs"],
            "violations": ["src/unscoped.rs"],
        }
    raise FixtureError(f"unsupported decide fix fixture kind: {kind}")


def decide_boulder_payload(kind: str) -> dict[str, Any]:
    if kind == "completed":
        return {
            "schema_version": 1,
            "stage": "grimoire-cast",
            "status": "completed",
            "boulder_required": True,
            "continuation_state": "completed",
            "semantic_iteration_cap": False,
        }
    require(kind == "skipped-clear-noop", f"unsupported decide boulder fixture kind: {kind}")
    return {
        "schema_version": 1,
        "stage": "grimoire-cast",
        "status": "skipped-clear-noop",
        "boulder_required": False,
        "continuation_state": "not-required",
        "semantic_iteration_cap": False,
    }


def decide_verdict_payload(kind: str) -> dict[str, Any]:
    statuses = {lens: "APPROVE" for lens in APPROVE_LENSES}
    if kind == "malformed":
        return {
            "schema_version": 1,
            "stage": "grimoire-verify",
            "approved": True,
            **statuses,
            "notes": {},
        }
    require(kind == "approve", f"unsupported decide verdict fixture kind: {kind}")
    return {
        "schema_version": 1,
        "stage": "grimoire-verify",
        "approved": True,
        **statuses,
        "notes": {lens: {"status": statuses[lens], "summary": "fixture", "evidence": ["synthetic fixture"]} for lens in APPROVE_LENSES},
    }


def decide_fixture_payloads(definition: dict[str, Any]) -> dict[str, dict[str, Any]]:
    return {
        DECIDE_INPUT_PATHS["preflight"]: decide_preflight_payload(str(definition["preflight"])),
        DECIDE_INPUT_PATHS["review"]: decide_review_payload(str(definition["review"])),
        DECIDE_INPUT_PATHS["design"]: decide_design_payload(str(definition["design"])),
        DECIDE_INPUT_PATHS["issues"]: decide_issues_payload(str(definition["issues"])),
        DECIDE_INPUT_PATHS["spec_gap"]: decide_spec_gap_payload(str(definition["spec_gap"])),
        DECIDE_INPUT_PATHS["fix"]: decide_fix_payload(str(definition["fix"])),
        DECIDE_INPUT_PATHS["boulder"]: decide_boulder_payload(str(definition["boulder"])),
        DECIDE_INPUT_PATHS["verdict"]: decide_verdict_payload(str(definition["verdict"])),
    }


def write_decide_static_fixtures() -> None:
    for definition in DECIDE_CASE_DEFINITIONS:
        case_dir = DECIDE_FIXTURE_ROOT / str(definition["name"])
        for relative, payload in decide_fixture_payloads(definition).items():
            write_json(case_dir / relative, payload)


def assert_decide_fixture_inventory(case: dict[str, Any]) -> None:
    case_dir = pathlib.Path(str(case["fixture_dir"]))
    require(case_dir.is_dir(), f"missing decide fixture directory: {case_dir}")
    expected = sorted(DECIDE_INPUT_PATHS.values())
    actual = sorted(path.relative_to(case_dir).as_posix() for path in case_dir.rglob("*") if path.is_file())
    require(actual == expected, f"{case['name']} fixture inventory drifted: actual={actual} expected={expected}")
    for relative in actual:
        require(relative.endswith(".json"), f"{case['name']} fixture contains a non-JSON input: {relative}")
        content = (case_dir / relative).read_text(encoding="utf-8")
        require(sanitize(content) == content, f"{case['name']} fixture contains a token-like secret pattern: {relative}")
        read_json(case_dir, relative)


def copy_decide_fixture_workspace(root: pathlib.Path, case: dict[str, Any]) -> pathlib.Path:
    source = pathlib.Path(str(case["fixture_dir"]))
    workspace = root / "decide-workspaces" / str(case["name"])
    if workspace.exists():
        shutil.rmtree(workspace)
    shutil.copytree(source, workspace)
    return workspace


def actual_decide_tuple(decision: dict[str, Any]) -> tuple[str, str, int, str]:
    return (
        str(decision.get("decision")),
        str(decision.get("conclusion")),
        int(decision.get("exit_code", 1)),
        str(decision.get("label_transition")),
    )


def run_decide_fixture_case(root: pathlib.Path, case: dict[str, Any], log_lines: list[str]) -> dict[str, Any]:
    assert_decide_fixture_inventory(case)
    workspace = copy_decide_fixture_workspace(root, case)
    run_helper(
        "cast",
        "cast_driver.py",
        [
            "decide",
            "--consumer-workspace",
            str(workspace),
            "--preflight-status",
            DECIDE_INPUT_PATHS["preflight"],
            "--review-status",
            DECIDE_INPUT_PATHS["review"],
            "--review-outcome",
            str(case["review_outcome"]),
            "--design-status",
            DECIDE_INPUT_PATHS["design"],
            "--issue-status",
            DECIDE_INPUT_PATHS["issues"],
            "--spec-gap-status",
            DECIDE_INPUT_PATHS["spec_gap"],
            "--fix-status",
            DECIDE_INPUT_PATHS["fix"],
            "--fix-outcome",
            str(case["fix_outcome"]),
            "--boulder-status",
            DECIDE_INPUT_PATHS["boulder"],
            "--verdict-status",
            DECIDE_INPUT_PATHS["verdict"],
            "--verify-outcome",
            str(case["verify_outcome"]),
            "--output",
            DECIDE_OUTPUT,
        ],
        {0},
        workspace,
        log_lines,
    )
    decision = read_json(workspace, DECIDE_OUTPUT)
    actual = actual_decide_tuple(decision)
    expected = case["expected"]
    require(actual == expected, f"{case['name']} tuple mismatch: actual={actual} expected={expected}")
    return {"name": case["name"], "label": case["label"], "actual": actual, "expected": expected}


def run_decide_self_check(root: pathlib.Path, log_path: pathlib.Path | None = None) -> dict[str, Any]:
    root.mkdir(parents=True, exist_ok=True)
    log_lines = [
        "# Grimoire Task 4 Decide Fixtures",
        f"generated_at={utc_now()}",
        f"repo_root={REPO_ROOT}",
        "secret_values=redacted",
        "",
    ]
    cases = load_decide_fixture_cases()
    summaries = [run_decide_fixture_case(root, case, log_lines) for case in cases]
    manifest = {
        "schema_version": 1,
        "generated_at": utc_now(),
        "fixture_count": len(summaries),
        "fixtures": summaries,
        "expected_tuples": expected_decide_tuples(),
        "real_remote_mutation_attempted": False,
        "secret_values_redacted": True,
        "model_output_dependency": False,
    }
    if log_path is not None:
        log_path.parent.mkdir(parents=True, exist_ok=True)
        log_path.write_text("\n".join(log_lines).rstrip() + "\n", encoding="utf-8")
    return manifest


def helper(stage: str, script_name: str) -> pathlib.Path:
    path = ACTIONS_ROOT / stage / "scripts" / script_name
    require(path.is_file(), f"missing helper: {path}")
    return path


def test_env(workspace: pathlib.Path) -> dict[str, str]:
    env = {
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": str(workspace),
        "TMPDIR": str(TEMP_ROOT),
        "GITHUB_WORKSPACE": str(workspace),
        "OPENCODE_DISABLE_PROJECT_CONFIG": "1",
        "OPENCODE_PURE": "1",
    }
    env.update(SENTINEL_ENV)
    return env


def run_helper(
    stage: str,
    script_name: str,
    args: list[str],
    expected: set[int],
    workspace: pathlib.Path,
    log_lines: list[str],
) -> subprocess.CompletedProcess[str]:
    script = helper(stage, script_name)
    command = ["python3", str(script), *args]
    result = subprocess.run(
        command,
        cwd=str(workspace),
        env=test_env(workspace),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    command_label = " ".join([str(script.relative_to(REPO_ROOT)), *args])
    log_lines.append(f"$ python3 {sanitize(command_label)}")
    log_lines.append(f"exit={result.returncode}")
    if result.stdout.strip():
        log_lines.append("stdout:")
        log_lines.append(sanitize(result.stdout).rstrip())
    if result.stderr.strip():
        log_lines.append("stderr:")
        log_lines.append(sanitize(result.stderr).rstrip())
    log_lines.append("")
    if result.returncode not in expected:
        raise FixtureError(
            "unexpected helper exit "
            f"{result.returncode}, expected {sorted(expected)} for {command_label}\n"
            f"stdout={sanitize(result.stdout)}\nstderr={sanitize(result.stderr)}"
        )
    return result


def finding(title: str, file_name: str = "src/lib.rs", severity: str = "medium") -> dict[str, Any]:
    return {
        "file": file_name,
        "line": 7,
        "severity": severity,
        "lens": "correctness",
        "title": title,
        "what": f"{title} was observed in the deterministic loop fixture.",
        "why": "The fixture needs a stable finding to exercise fail-closed loop semantics.",
        "suggested_fix": "Bind the finding to OpenSpec before allowing fixes.",
        "evidence": f"{file_name}:7 deterministic fixture evidence",
    }


def write_trusted(workspace: pathlib.Path) -> None:
    write_json(
        workspace / ".omo" / "ci" / "trusted-controller-status.json",
        {
            "schema_version": 1,
            "stage": "grimoire-trusted-controller",
            "status": "ok",
            "action": "continue",
            "model_execution_allowed": True,
            "write_allowed": True,
            "commit_allowed": True,
            "push_allowed": True,
            "github_mutation_allowed": True,
        },
    )


def run_preflight(workspace: pathlib.Path, log_lines: list[str]) -> str:
    write_trusted(workspace)
    output = ".omo/ci/cast-preflight.json"
    run_helper(
        "cast",
        "cast_driver.py",
        [
            "preflight",
            "--consumer-workspace",
            str(workspace),
            "--trusted-status-path",
            ".omo/ci/trusted-controller-status.json",
            "--trusted-outcome",
            "success",
            "--trusted-status",
            "ok",
            "--trusted-action",
            "continue",
            "--model-execution-allowed",
            "true",
            "--write-allowed",
            "true",
            "--commit-allowed",
            "true",
            "--push-allowed",
            "true",
            "--github-mutation-allowed",
            "true",
            "--output",
            output,
        ],
        {0},
        workspace,
        log_lines,
    )
    preflight = read_json(workspace, output)
    require(preflight.get("can_continue") is True, "preflight must allow deterministic fixture continuation")
    return output


def run_review_clean(workspace: pathlib.Path, output: str, log_lines: list[str]) -> str:
    run_helper(
        "review",
        "review.py",
        ["--consumer-workspace", str(workspace), "--fixture", "clean", "--output", output],
        {0},
        workspace,
        log_lines,
    )
    review = read_json(workspace, output)
    require(review.get("status") == "approved", "clean review fixture must approve")
    return output


def run_review_defect(workspace: pathlib.Path, output: str, log_lines: list[str], severity: str = "medium") -> str:
    diff_path = write_text(
        workspace / "defect.diff",
        "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -7,1 +7,1 @@\n+fn fixture_defect() { /* GRIMOIRE_REVIEW_DEFECT */ }\n",
    )
    run_helper(
        "review",
        "review.py",
        ["--consumer-workspace", str(workspace), "--fixture", "defect", "--fixture-input", diff_path.name, "--output", output],
        {0},
        workspace,
        log_lines,
    )
    review = read_json(workspace, output)
    findings = review.get("findings")
    if isinstance(findings, list):
        for item in cast(list[object], findings):
            if isinstance(item, dict):
                item["severity"] = severity
    write_json(workspace / output, review)
    require(review.get("status") == "findings", "defect review fixture must produce findings")
    return output


def run_design(workspace: pathlib.Path, review: str, output: str, plan: str, has_spec: bool, log_lines: list[str]) -> str:
    if has_spec:
        write_text(
            workspace / "openspec" / "specs" / "grimoire-fixture.md",
            "### Requirement: Deterministic review defect marker present\nThe fixture behavior is intentionally covered.\n",
        )
    run_helper(
        "design",
        "design.py",
        [
            "--consumer-workspace",
            str(workspace),
            "--repository",
            "local-consumer",
            "--review-input",
            review,
            "--output",
            output,
            "--plan",
            plan,
        ],
        {0} if has_spec or "review-clean" in review else {1},
        workspace,
        log_lines,
    )
    return output


def run_spec_gap(workspace: pathlib.Path, design: str, status: str, comment: str, expected: set[int], log_lines: list[str]) -> str:
    run_helper(
        "spec-gap",
        "spec_gap.py",
        [
            "--consumer-workspace",
            str(workspace),
            "--input",
            design,
            "--comment-output",
            comment,
            "--status-output",
            status,
        ],
        expected,
        workspace,
        log_lines,
    )
    return status


def run_issues(workspace: pathlib.Path, design: str, output: str, log_lines: list[str]) -> str:
    run_helper(
        "cast",
        "cast_driver.py",
        [
            "file-issues",
            "--consumer-workspace",
            str(workspace),
            "--design-path",
            design,
            "--repository",
            "local-consumer",
            "--pr-number",
            "0",
            "--ledger",
            ".omo/ci/out-of-scope-issues-ledger.json",
            "--output",
            output,
        ],
        {0},
        workspace,
        log_lines,
    )
    issues = read_json(workspace, output)
    require(issues.get("status") == "ok", "local issue intent fixture must complete without network")
    return output


def run_fix_clear(workspace: pathlib.Path, design: str, gap: str, output: str, handoff: str, log_lines: list[str]) -> str:
    run_helper(
        "fix",
        "fix.py",
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            design,
            "--spec-gap-status",
            gap,
            "--output",
            output,
            "--handoff-output",
            handoff,
        ],
        {0},
        workspace,
        log_lines,
    )
    fix = read_json(workspace, output)
    require(fix.get("status") == "clear-noop" and fix.get("should_push") is False, "clear fix must be a no-push noop")
    return output


def run_fix_fixed(workspace: pathlib.Path, design: str, gap: str, output: str, handoff: str, log_lines: list[str]) -> str:
    write_text(workspace / "pr-touched.txt", "src/lib.rs\n")
    write_text(workspace / "changed-fixed.txt", "src/lib.rs\n")
    run_helper(
        "fix",
        "fix.py",
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            design,
            "--spec-gap-status",
            gap,
            "--pr-touched",
            "pr-touched.txt",
            "--changed-files",
            "changed-fixed.txt",
            "--output",
            output,
            "--handoff-output",
            handoff,
        ],
        {0},
        workspace,
        log_lines,
    )
    fix = read_json(workspace, output)
    require(fix.get("status") == "fixed" and fix.get("should_push") is True, "fixed fixture must request later scoped push")
    return output


def run_verify(workspace: pathlib.Path, output: str, fixture: str, expected: set[int], log_lines: list[str]) -> str:
    run_helper(
        "verify",
        "verify.py",
        ["--consumer-workspace", str(workspace), "--fixture", fixture, "--output", output],
        expected,
        workspace,
        log_lines,
    )
    return output


def run_boulder(workspace: pathlib.Path, fix: str, output: str, expected: set[int], log_lines: list[str]) -> str:
    run_helper(
        "cast",
        "cast_driver.py",
        ["boulder", "--consumer-workspace", str(workspace), "--fix-status", fix, "--output", output],
        expected,
        workspace,
        log_lines,
    )
    return output


def run_decide(
    workspace: pathlib.Path,
    output: str,
    preflight: str,
    review: str,
    design: str,
    issues: str,
    spec_gap: str,
    fix: str,
    boulder: str,
    verdict: str,
    verify_outcome: str,
    log_lines: list[str],
) -> str:
    run_helper(
        "cast",
        "cast_driver.py",
        [
            "decide",
            "--consumer-workspace",
            str(workspace),
            "--preflight-status",
            preflight,
            "--review-status",
            review,
            "--review-outcome",
            "success",
            "--design-status",
            design,
            "--issue-status",
            issues,
            "--spec-gap-status",
            spec_gap,
            "--fix-status",
            fix,
            "--fix-outcome",
            "success",
            "--boulder-status",
            boulder,
            "--verdict-status",
            verdict,
            "--verify-outcome",
            verify_outcome,
            "--output",
            output,
        ],
        {0},
        workspace,
        log_lines,
    )
    return output


def run_complete(workspace: pathlib.Path, decision: str, output: str, expected: set[int], log_lines: list[str], push_status: str | None = None) -> str:
    args = ["complete", "--consumer-workspace", str(workspace), "--decision", decision, "--output", output]
    if push_status is not None:
        args.extend(["--push-status", push_status])
    run_helper("cast", "cast_driver.py", args, expected, workspace, log_lines)
    return output


def prepare_workspace(root: pathlib.Path, name: str) -> pathlib.Path:
    workspace = root / "workspaces" / name
    workspace.mkdir(parents=True, exist_ok=True)
    return workspace


def summarize(workspace: pathlib.Path, root: pathlib.Path, name: str, fields: dict[str, Any]) -> dict[str, Any]:
    summary = {
        "schema_version": 1,
        "fixture": name,
        "workspace": workspace.relative_to(root).as_posix(),
        "generated_at": utc_now(),
        "real_remote_mutation_attempted": False,
        "sentinel_values_redacted": True,
    }
    summary.update(fields)
    write_json(root / "summaries" / f"{name}.json", summary)
    return summary


def clear_noop_fixture(root: pathlib.Path, log_lines: list[str]) -> dict[str, Any]:
    workspace = prepare_workspace(root, "clear-noop")
    preflight = run_preflight(workspace, log_lines)
    review = run_review_clean(workspace, ".omo/ci/review-clean.json", log_lines)
    design = run_design(workspace, review, ".omo/ci/spec-clear.json", ".omo/ci/design-clear.md", False, log_lines)
    gap = run_spec_gap(workspace, design, ".omo/ci/spec-gap-clear.json", ".omo/ci/spec-gap-clear.md", {0}, log_lines)
    issues = run_issues(workspace, design, ".omo/ci/issues-clear.json", log_lines)
    fix = ".omo/ci/fix-clear.json"
    write_json(workspace / fix, decide_fix_payload("clear-noop"))
    boulder = run_boulder(workspace, fix, ".omo/boulder-clear.json", {0}, log_lines)
    verdict = run_verify(workspace, ".omo/grimoire/verdict-approve.json", "approve", {0}, log_lines)
    decision_path = run_decide(
        workspace,
        ".omo/ci/cast-decision-clear.json",
        preflight,
        review,
        design,
        issues,
        gap,
        fix,
        boulder,
        verdict,
        "success",
        log_lines,
    )
    final_path = run_complete(workspace, decision_path, ".omo/ci/cast-final-clear.json", {0}, log_lines)
    decision = read_json(workspace, decision_path)
    final = read_json(workspace, final_path)
    require(decision.get("decision") == "no-actionable-work" and decision.get("conclusion") == "neutral", "clean review with no actionable work must decide neutral no-actionable")
    require(decision.get("terminal") is False and decision.get("should_push") is False, "no-actionable work must not terminate as Cast or push")
    require(final.get("status") == "advisory" and final.get("conclusion") == "neutral", "no-actionable complete must be advisory")
    return summarize(
        workspace,
        root,
        "clear-noop",
        {
            "decision": decision.get("decision"),
            "conclusion": decision.get("conclusion"),
            "exit_code": decision.get("exit_code"),
            "final_conclusion": final.get("conclusion"),
            "terminal": decision.get("terminal"),
            "should_push": decision.get("should_push"),
            "final_status": final.get("status"),
            "push_intent_recorded": False,
        },
    )


def fixed_then_clear_fixture(root: pathlib.Path, log_lines: list[str]) -> dict[str, Any]:
    workspace = prepare_workspace(root, "fixed-then-clear")
    preflight = run_preflight(workspace, log_lines)
    review = run_review_defect(workspace, ".omo/ci/review-defect.json", log_lines, severity="high")
    design = run_design(workspace, review, ".omo/ci/spec-sufficient.json", ".omo/ci/design-sufficient.md", True, log_lines)
    gap = run_spec_gap(workspace, design, ".omo/ci/spec-gap-clear.json", ".omo/ci/spec-gap-clear.md", {0}, log_lines)
    issues = run_issues(workspace, design, ".omo/ci/issues-fixed.json", log_lines)
    fixed = run_fix_fixed(workspace, design, gap, ".omo/ci/fix-fixed.json", ".omo/ci/fix-fixed.md", log_lines)
    boulder_fixed = run_boulder(workspace, fixed, ".omo/boulder-fixed.json", {0}, log_lines)
    approve = run_verify(workspace, ".omo/grimoire/verdict-approve.json", "approve", {0}, log_lines)
    fixed_decision_path = run_decide(
        workspace,
        ".omo/ci/cast-decision-fixed.json",
        preflight,
        review,
        design,
        issues,
        gap,
        fixed,
        boulder_fixed,
        approve,
        "success",
        log_lines,
    )
    fixed_decision = read_json(workspace, fixed_decision_path)
    require(fixed_decision.get("decision") == "scoped-push", "fixed cycle must request scoped-push intent")
    require(fixed_decision.get("should_push") is True and fixed_decision.get("terminal") is False, "fixed cycle must await synchronize")
    synthetic_push_status = ".omo/ci/cast-push-status-intent.json"
    write_json(
        workspace / synthetic_push_status,
        {
            "schema_version": 1,
            "stage": "grimoire-cast",
            "status": "pushed",
            "push_count": 1,
            "push_attempted": False,
            "local_intent_only": True,
            "next_expected_event": "pull_request.synchronize",
        },
    )
    fixed_final_path = run_complete(
        workspace,
        fixed_decision_path,
        ".omo/ci/cast-final-fixed.json",
        {0},
        log_lines,
        synthetic_push_status,
    )
    fixed_final = read_json(workspace, fixed_final_path)
    require(fixed_final.get("status") == "awaiting-synchronize", "fixed cycle must await synchronize re-review")

    review_clear = run_review_clean(workspace, ".omo/ci/review-clean-cycle2.json", log_lines)
    design_clear = run_design(workspace, review_clear, ".omo/ci/spec-clear-cycle2.json", ".omo/ci/design-clear-cycle2.md", False, log_lines)
    gap_clear = run_spec_gap(workspace, design_clear, ".omo/ci/spec-gap-clear-cycle2.json", ".omo/ci/spec-gap-clear-cycle2.md", {0}, log_lines)
    issues_clear = run_issues(workspace, design_clear, ".omo/ci/issues-clear-cycle2.json", log_lines)
    clear = run_fix_clear(workspace, design_clear, gap_clear, ".omo/ci/fix-clear-cycle2.json", ".omo/ci/fix-clear-cycle2.md", log_lines)
    boulder_clear = run_boulder(workspace, clear, ".omo/boulder-clear-cycle2.json", {0}, log_lines)
    clear_decision_path = run_decide(
        workspace,
        ".omo/ci/cast-decision-clear-cycle2.json",
        preflight,
        review_clear,
        design_clear,
        issues_clear,
        gap_clear,
        clear,
        boulder_clear,
        approve,
        "success",
        log_lines,
    )
    clear_final_path = run_complete(workspace, clear_decision_path, ".omo/ci/cast-final-clear-cycle2.json", {0}, log_lines)
    clear_decision = read_json(workspace, clear_decision_path)
    clear_final = read_json(workspace, clear_final_path)
    require(clear_decision.get("decision") == "no-actionable-work" and clear_decision.get("conclusion") == "neutral", "synchronize re-review with no actionable work must be neutral advisory")
    require(clear_final.get("status") == "advisory" and clear_final.get("conclusion") == "neutral", "synchronize clean cycle must complete advisory")
    return summarize(
        workspace,
        root,
        "fixed-then-clear",
        {
            "cycle1_decision": fixed_decision.get("decision"),
            "cycle1_should_push": fixed_decision.get("should_push"),
            "cycle1_final_status": fixed_final.get("status"),
            "cycle1_real_push_attempted": False,
            "cycle2_decision": clear_decision.get("decision"),
            "cycle2_terminal": clear_decision.get("terminal"),
            "cycle2_should_push": clear_decision.get("should_push"),
            "cycle2_final_status": clear_final.get("status"),
            "push_intent_recorded": True,
        },
    )


def spec_insufficient_fixture(root: pathlib.Path, log_lines: list[str]) -> dict[str, Any]:
    workspace = prepare_workspace(root, "spec-insufficient")
    preflight = run_preflight(workspace, log_lines)
    review = run_review_defect(workspace, ".omo/ci/review-defect.json", log_lines, severity="high")
    design = run_design(workspace, review, ".omo/ci/spec-insufficient.json", ".omo/ci/design-insufficient.md", False, log_lines)
    gap = run_spec_gap(workspace, design, ".omo/ci/spec-gap-halt.json", ".omo/ci/spec-gap-comment.md", {0}, log_lines)
    issues = run_issues(workspace, design, ".omo/ci/issues-insufficient.json", log_lines)
    approve = run_verify(workspace, ".omo/grimoire/verdict-approve.json", "approve", {0}, log_lines)
    decision_path = run_decide(
        workspace,
        ".omo/ci/cast-decision-spec-insufficient.json",
        preflight,
        review,
        design,
        issues,
        gap,
        ".omo/ci/fix-not-run.json",
        ".omo/boulder-not-run.json",
        approve,
        "success",
        log_lines,
    )
    final_path = run_complete(workspace, decision_path, ".omo/ci/cast-final-spec-insufficient.json", {0}, log_lines)
    gap_payload = read_json(workspace, gap)
    decision = read_json(workspace, decision_path)
    final = read_json(workspace, final_path)
    require(gap_payload.get("status") == "halt" and gap_payload.get("no_code_or_push_action") is True, "spec-gap halt must forbid code or push action")
    require(decision.get("decision") == "spec-gap-halt" and decision.get("conclusion") == "neutral" and decision.get("exit_code") == 0, "spec-insufficient path must complete as neutral advisory")
    require(final.get("status") == "advisory" and final.get("conclusion") == "neutral", "spec-insufficient complete must be advisory")
    require(not (workspace / ".omo" / "ci" / "fix-not-run.json").exists(), "spec-insufficient path must not write a fix artifact")
    return summarize(
        workspace,
        root,
        "spec-insufficient",
        {
            "spec_gap_status": gap_payload.get("status"),
            "decision": decision.get("decision"),
            "conclusion": decision.get("conclusion"),
            "exit_code": decision.get("exit_code"),
            "final_status": final.get("status"),
            "final_conclusion": final.get("conclusion"),
            "terminal": decision.get("terminal"),
            "should_push": decision.get("should_push"),
            "code_write_attempted": False,
            "push_intent_recorded": False,
        },
    )


def reject_fixture(root: pathlib.Path, log_lines: list[str]) -> dict[str, Any]:
    workspace = prepare_workspace(root, "reject")
    preflight = run_preflight(workspace, log_lines)
    review = run_review_defect(workspace, ".omo/ci/review-defect.json", log_lines, severity="high")
    design = run_design(workspace, review, ".omo/ci/spec-clear.json", ".omo/ci/design-clear.md", True, log_lines)
    gap = run_spec_gap(workspace, design, ".omo/ci/spec-gap-clear.json", ".omo/ci/spec-gap-clear.md", {0}, log_lines)
    issues = run_issues(workspace, design, ".omo/ci/issues-reject.json", log_lines)
    fix = ".omo/ci/fix-clear.json"
    write_json(workspace / fix, decide_fix_payload("clear-noop"))
    boulder = run_boulder(workspace, fix, ".omo/boulder-clear.json", {0}, log_lines)
    reject = run_verify(workspace, ".omo/grimoire/verdict-reject.json", "reject", {1}, log_lines)
    decision_path = run_decide(
        workspace,
        ".omo/ci/cast-decision-reject.json",
        preflight,
        review,
        design,
        issues,
        gap,
        fix,
        boulder,
        reject,
        "failure",
        log_lines,
    )
    final_path = run_complete(workspace, decision_path, ".omo/ci/cast-final-reject.json", {1}, log_lines)
    verdict = read_json(workspace, reject)
    decision = read_json(workspace, decision_path)
    final = read_json(workspace, final_path)
    require(verdict.get("approved") is False, "reject verdict must not approve")
    require(decision.get("status") == "fizzled" and decision.get("should_push") is False, "reject path must fizzle without push")
    require(final.get("status") == "fizzled", "reject complete must fizzle")
    return summarize(
        workspace,
        root,
        "reject",
        {
            "verdict_approved": verdict.get("approved"),
            "decision": decision.get("decision"),
            "conclusion": decision.get("conclusion"),
            "exit_code": decision.get("exit_code"),
            "final_status": final.get("status"),
            "final_conclusion": final.get("conclusion"),
            "terminal": decision.get("terminal"),
            "should_push": decision.get("should_push"),
            "code_write_attempted": False,
            "push_intent_recorded": False,
        },
    )


def boulder_incomplete_fixture(root: pathlib.Path, log_lines: list[str]) -> dict[str, Any]:
    workspace = prepare_workspace(root, "boulder-incomplete")
    preflight = run_preflight(workspace, log_lines)
    review = run_review_defect(workspace, ".omo/ci/review-defect.json", log_lines)
    design = run_design(workspace, review, ".omo/ci/spec-sufficient.json", ".omo/ci/design-sufficient.md", True, log_lines)
    gap = run_spec_gap(workspace, design, ".omo/ci/spec-gap-clear.json", ".omo/ci/spec-gap-clear.md", {0}, log_lines)
    issues = run_issues(workspace, design, ".omo/ci/issues-boulder.json", log_lines)
    fix = run_fix_fixed(workspace, design, gap, ".omo/ci/fix-fixed.json", ".omo/ci/fix-fixed.md", log_lines)
    approve = run_verify(workspace, ".omo/grimoire/verdict-approve.json", "approve", {0}, log_lines)
    boulder = ".omo/boulder-incomplete.json"
    write_json(
        workspace / boulder,
        {
            "schema_version": 1,
            "stage": "grimoire-cast",
            "status": "blocked",
            "boulder_required": True,
            "continuation_state": "incomplete",
            "push_attempted": False,
        },
    )
    decision_path = run_decide(
        workspace,
        ".omo/ci/cast-decision-boulder-incomplete.json",
        preflight,
        review,
        design,
        issues,
        gap,
        fix,
        boulder,
        approve,
        "success",
        log_lines,
    )
    final_path = run_complete(workspace, decision_path, ".omo/ci/cast-final-boulder-incomplete.json", {1}, log_lines)
    decision = read_json(workspace, decision_path)
    final = read_json(workspace, final_path)
    require(decision.get("status") == "fizzled", "incomplete boulder state must fizzle")
    require(decision.get("terminal") is False and decision.get("should_push") is False, "incomplete boulder must not terminate or push")
    require(final.get("status") == "fizzled", "incomplete boulder complete must fizzle")
    return summarize(
        workspace,
        root,
        "boulder-incomplete",
        {
            "decision": decision.get("decision"),
            "final_status": final.get("status"),
            "terminal": decision.get("terminal"),
            "should_push": decision.get("should_push"),
            "boulder_continuation_state": "incomplete",
            "push_intent_recorded": False,
        },
    )


def run_all(root: pathlib.Path, log_path: pathlib.Path) -> dict[str, Any]:
    root.mkdir(parents=True, exist_ok=True)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_lines = [
        "# Grimoire Task 5 Full Loop Fixtures",
        f"generated_at={utc_now()}",
        f"repo_root={REPO_ROOT}",
        "secret_values=redacted",
        "",
    ]
    summaries = [
        clear_noop_fixture(root, log_lines),
        fixed_then_clear_fixture(root, log_lines),
        spec_insufficient_fixture(root, log_lines),
        reject_fixture(root, log_lines),
        boulder_incomplete_fixture(root, log_lines),
    ]
    manifest = {
        "schema_version": 1,
        "generated_at": utc_now(),
        "artifact_root": str(root),
        "log_path": str(log_path),
        "fixture_count": len(summaries),
        "fixtures": summaries,
        "real_remote_mutation_attempted": False,
        "secret_values_redacted": True,
        "runtime_workflow_referenced": False,
    }
    write_json(root / "manifest.json", manifest)
    log_path.write_text("\n".join(log_lines).rstrip() + "\n", encoding="utf-8")
    return manifest


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run deterministic Grimoire loop fixtures without live mutation.")
    parser.add_argument("--artifact-root", type=pathlib.Path, default=None)
    parser.add_argument("--log", type=pathlib.Path, default=None)
    parser.add_argument("--generate-decide-fixtures", action="store_true")
    parser.add_argument("--decide-self-check", action="store_true")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.generate_decide_fixtures:
        try:
            write_decide_static_fixtures()
        except FixtureError as exc:
            print(sanitize(str(exc)), file=sys.stderr)
            return 1
        print("decide fixtures generated")
        print(f"fixture_root={DECIDE_FIXTURE_ROOT}")
        print(f"fixture_count={len(DECIDE_CASE_DEFINITIONS)}")
        return 0

    TEMP_ROOT.mkdir(parents=True, exist_ok=True)
    artifact_root = args.artifact_root
    if args.decide_self_check:
        if artifact_root is None:
            artifact_root = pathlib.Path(tempfile.mkdtemp(prefix="grimoire-task-4-decide-fixtures-", dir=str(TEMP_ROOT)))
        artifact_root = ensure_temp_path(artifact_root, "artifact root")
        log_path = ensure_temp_path(args.log, "decide fixture log") if args.log is not None else None
        try:
            if artifact_root.exists() and any(artifact_root.iterdir()):
                raise FixtureError(f"artifact root must be empty before fixture run: {artifact_root}")
            manifest = run_decide_self_check(artifact_root, log_path)
        except FixtureError as exc:
            print(sanitize(str(exc)), file=sys.stderr)
            return 1
        print("decide fixtures ok")
        print(f"artifact_root={artifact_root}")
        print(f"fixture_count={manifest['fixture_count']}")
        for summary in manifest["fixtures"]:
            decision, conclusion, exit_code, label_transition = summary["actual"]
            print(
                f"PASS {summary['name']} ({summary['label']}): "
                f"decision={decision} conclusion={conclusion} exit_code={exit_code} label_transition={label_transition}"
            )
        if log_path is not None:
            print(f"log={log_path}")
        return 0

    if artifact_root is None:
        artifact_root = pathlib.Path(tempfile.mkdtemp(prefix="grimoire-task-5-fixtures-", dir=str(TEMP_ROOT)))
    artifact_root = ensure_temp_path(artifact_root, "artifact root")
    log_path = args.log or (artifact_root / "loop-fixtures.log")
    log_path = ensure_temp_path(log_path, "fixture log")
    try:
        if artifact_root.exists() and any(artifact_root.iterdir()):
            raise FixtureError(f"artifact root must be empty before fixture run: {artifact_root}")
        manifest = run_all(artifact_root, log_path)
    except FixtureError as exc:
        print(sanitize(str(exc)), file=sys.stderr)
        return 1
    print("loop fixtures ok")
    print(f"artifact_root={manifest['artifact_root']}")
    print(f"log={manifest['log_path']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
