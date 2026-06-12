# pyright: reportAny=false, reportExplicitAny=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


TEMP_ROOT = Path("/var/folders/vz/hx33c759727ftq88cxbgp8r40000gn/T/opencode")
LENSES = ["security", "correctness", "maintainability", "repo-policy"]
APPROVE_LENSES = ("f1_oracle", "f2_quality", "f3_real_qa", "f4_scope")
SPEC_GAP_SECTIONS = ["Summary", "Intended Work", "Missing OpenSpec Evidence", "Suggested Spec Items", "How To Rerun"]


class ContractError(AssertionError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def make_workspace(name: str) -> Path:
    TEMP_ROOT.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix=f"grimoire-{name}-", dir=str(TEMP_ROOT)))


def helper(actions_root: Path, stage: str, name: str) -> Path:
    path = actions_root / stage / "scripts" / name
    require(path.is_file(), f"missing helper: {path}")
    require(os.access(path, os.X_OK), f"helper is not executable: {path}")
    return path


def rel_path(workspace: Path, relative: str) -> Path:
    return workspace / relative


def write_json(workspace: Path, relative: str, payload: dict[str, Any]) -> Path:
    path = rel_path(workspace, relative)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return path


def read_json(workspace: Path, relative: str) -> dict[str, Any]:
    path = rel_path(workspace, relative)
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise ContractError(f"missing JSON artifact: {relative}") from exc
    except json.JSONDecodeError as exc:
        raise ContractError(f"malformed JSON artifact {relative}: {exc}") from exc
    require(isinstance(payload, dict), f"JSON artifact must be an object: {relative}")
    return payload


def run_helper(script: Path, args: list[str], expected: set[int], env: dict[str, str] | None = None, cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    command = ["python3", str(script), *args]
    result = subprocess.run(command, cwd=str(cwd or script.parent), env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if result.returncode not in expected:
        raise ContractError(
            f"command failed unexpectedly ({result.returncode}, expected {sorted(expected)}): {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result


def finding(title: str, path: str = "src/lib.rs", *, out_of_scope: bool = False) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "file": path,
        "line": 7,
        "severity": "medium",
        "lens": "correctness",
        "title": title,
        "what": f"{title} was observed in the deterministic contract fixture.",
        "why": "The contract test needs a stable stage artifact to validate loop semantics.",
        "suggested_fix": "Bind the finding to OpenSpec or track it outside the loop.",
        "evidence": f"{path}:7 deterministic fixture evidence",
    }
    if out_of_scope:
        payload["out_of_scope"] = True
        payload["scope"] = "out_of_scope"
    return payload


def write_review_artifact(workspace: Path, relative: str, findings: list[dict[str, Any]]) -> Path:
    status = "findings" if findings else "approved"
    return write_json(
        workspace,
        relative,
        {
            "schema_version": 1,
            "stage": "grimoire-review",
            "status": status,
            "approval_signal": "GRIMOIRE_REVIEW_FINDINGS_PRESENT" if findings else "GRIMOIRE_REVIEW_APPROVED",
            "read_only": True,
            "mutation_allowed": False,
            "findings": findings,
            "findings_count": len(findings),
            "lenses": LENSES,
        },
    )


def write_design_artifact(workspace: Path, relative: str, *, sufficient: bool, in_scope: list[dict[str, Any]] | None = None, bindings: list[dict[str, Any]] | None = None) -> Path:
    return write_json(
        workspace,
        relative,
        {
            "schema_version": 1,
            "stage": "grimoire-design",
            "status": "sufficient" if sufficient else "insufficient",
            "spec_sufficient": sufficient,
            "should_halt": not sufficient,
            "halt_reason": "" if sufficient else "deterministic spec gap",
            "scope_authority": "OpenSpec and OMO",
            "in_scope": in_scope or [],
            "out_of_scope": [],
            "bindings": bindings or [],
            "missing": [] if sufficient else [{"location": "src/lib.rs:7", "reason": "missing fixture evidence"}],
            "safety_default_gaps": [] if sufficient else [{"scope": "src/lib.rs:7", "required_default": "halt before fix"}],
            "suggested_spec_patch": "Add fixture OpenSpec evidence." if not sufficient else "",
            "plan_path": ".omo/ci/design-plan.md",
        },
    )


def write_fix_artifact(workspace: Path, relative: str, status: str) -> Path:
    scope_ok = status in {"clear-noop", "fixed"}
    return write_json(
        workspace,
        relative,
        {
            "schema_version": 1,
            "stage": "grimoire-fix",
            "status": status,
            "scope_ok": scope_ok,
            "noop": status == "clear-noop",
            "should_commit": status == "fixed",
            "should_push": status == "fixed",
            "changed_files": [] if status == "clear-noop" else ["src/lib.rs"],
        },
    )


def write_verdict(workspace: Path, relative: str, approved: bool) -> Path:
    statuses = {lens: "APPROVE" if approved else "REJECT" for lens in APPROVE_LENSES}
    return write_json(
        workspace,
        relative,
        {
            "schema_version": 1,
            "stage": "grimoire-verify",
            "approved": approved,
            **statuses,
            "notes": {lens: {"status": statuses[lens], "summary": "fixture", "evidence": ["fixture"]} for lens in APPROVE_LENSES},
        },
    )


def assert_review(actions_root: Path, workspace: Path) -> None:
    script = helper(actions_root, "review", "review.py")
    run_helper(script, ["--consumer-workspace", str(workspace), "--output", ".omo/ci/review-clean.json", "--fixture", "clean"], {0})
    clean = read_json(workspace, ".omo/ci/review-clean.json")
    require(clean["status"] == "approved", "clean review fixture must approve")
    require(clean["read_only"] is True and clean["mutation_allowed"] is False, "review must remain read-only and mutation-free")
    require(clean["lenses"] == LENSES, "review lenses must remain the recovered four-lens set")

    fixture = rel_path(workspace, "defect.diff")
    fixture.write_text("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,1 +1,1 @@\n+fn bad() { /* GRIMOIRE_REVIEW_DEFECT */ }\n", encoding="utf-8")
    run_helper(
        script,
        ["--consumer-workspace", str(workspace), "--output", ".omo/ci/review-defect.json", "--fixture", "defect", "--fixture-input", "defect.diff"],
        {0},
    )
    defect = read_json(workspace, ".omo/ci/review-defect.json")
    require(defect["status"] == "findings" and defect["findings_count"] == 1, "defect review fixture must emit one finding")
    require(defect["read_only"] is True and defect["mutation_allowed"] is False, "defect review must still be read-only")


def assert_design_and_issues(actions_root: Path, workspace: Path) -> None:
    design_script = helper(actions_root, "design", "design.py")
    cast_script = helper(actions_root, "cast", "cast_driver.py")
    write_review_artifact(
        workspace,
        ".omo/ci/review-design.json",
        [finding("In-scope deterministic defect"), finding("[out-of-scope] Separate infra cleanup", "infra/cleanup.md", out_of_scope=True)],
    )
    args = [
        "--consumer-workspace",
        str(workspace),
        "--repository",
        "local-consumer",
        "--review-input",
        ".omo/ci/review-design.json",
        "--output",
        ".omo/ci/spec-insufficient.json",
        "--plan",
        ".omo/ci/design-insufficient.md",
    ]
    run_helper(design_script, args, {1})
    insufficient = read_json(workspace, ".omo/ci/spec-insufficient.json")
    require(insufficient["status"] == "insufficient" and insufficient["should_halt"] is True, "design must halt when in-scope OpenSpec evidence is missing")
    require(len(insufficient["in_scope"]) == 1 and len(insufficient["out_of_scope"]) == 1, "design must split in_scope and out_of_scope findings")
    first_fingerprint = insufficient["out_of_scope"][0]["fingerprint"]
    require(re.fullmatch(r"[0-9a-f]{24}", first_fingerprint) is not None, "out-of-scope fingerprint must be stable 24-char hex")
    run_helper(design_script, args[:-4] + ["--output", ".omo/ci/spec-insufficient-repeat.json", "--plan", ".omo/ci/design-repeat.md"], {1})
    repeated = read_json(workspace, ".omo/ci/spec-insufficient-repeat.json")
    require(repeated["out_of_scope"][0]["fingerprint"] == first_fingerprint, "out-of-scope fingerprint must be stable across runs")

    run_helper(
        cast_script,
        [
            "file-issues",
            "--consumer-workspace",
            str(workspace),
            "--design-path",
            ".omo/ci/spec-insufficient.json",
            "--repository",
            "local-consumer",
            "--pr-number",
            "0",
            "--output",
            ".omo/ci/issues-first.json",
            "--ledger",
            ".omo/ci/issues-ledger.json",
        ],
        {0},
    )
    first = read_json(workspace, ".omo/ci/issues-first.json")
    require(first["status"] == "ok" and first["issue_write_only"] is True, "out-of-scope issue filing must be Issues-only")
    require(first["design_stage_complete"] is True and first["pr_head_write_attempted"] is False, "issue filing must occur only after design and not write PR-head files")
    require(first["records"][0]["status"] == "recorded-local-intent", "local issue fixture must record intent without network")
    run_helper(
        cast_script,
        [
            "file-issues",
            "--consumer-workspace",
            str(workspace),
            "--design-path",
            ".omo/ci/spec-insufficient.json",
            "--repository",
            "local-consumer",
            "--pr-number",
            "0",
            "--output",
            ".omo/ci/issues-repeat.json",
            "--ledger",
            ".omo/ci/issues-ledger.json",
        ],
        {0},
    )
    repeat = read_json(workspace, ".omo/ci/issues-repeat.json")
    require(repeat["deduped_count"] == 1 and repeat["records"][0]["status"] == "deduped-local", "out-of-scope issue filing must dedupe by fingerprint")

    write_json(workspace, ".omo/ci/not-design.json", {"schema_version": 1, "stage": "not-design", "out_of_scope": []})
    run_helper(
        cast_script,
        ["file-issues", "--consumer-workspace", str(workspace), "--design-path", ".omo/ci/not-design.json", "--repository", "local-consumer", "--pr-number", "0", "--output", ".omo/ci/issues-not-design.json"],
        {1},
    )
    not_design = read_json(workspace, ".omo/ci/issues-not-design.json")
    require(not_design["status"] == "blocked", "out-of-scope filing must reject non-design artifacts")


def assert_spec_gap(actions_root: Path, workspace: Path) -> None:
    script = helper(actions_root, "spec-gap", "spec_gap.py")
    run_helper(
        script,
        ["--consumer-workspace", str(workspace), "--input", ".omo/ci/spec-insufficient.json", "--comment-output", ".omo/ci/spec-gap-comment.md", "--status-output", ".omo/ci/spec-gap-status.json"],
        {0},
    )
    status = read_json(workspace, ".omo/ci/spec-gap-status.json")
    require(status["status"] == "halt" and status["should_halt"] is True and status["should_comment"] is True, "spec-gap must emit halt status")
    require(status["top_level_sections"] == SPEC_GAP_SECTIONS and status["top_level_section_count"] == 5, "spec-gap must expose exactly five top-level sections")
    comment = rel_path(workspace, ".omo/ci/spec-gap-comment.md").read_text(encoding="utf-8")
    headings = [line.removeprefix("## ") for line in comment.splitlines() if line.startswith("## ")]
    require(headings == SPEC_GAP_SECTIONS, "spec-gap comment headings drifted")


def assert_fix(actions_root: Path, workspace: Path) -> tuple[str, str]:
    script = helper(actions_root, "fix", "fix.py")
    write_json(workspace, ".omo/ci/spec-gap-clear.json", {"schema_version": 1, "stage": "grimoire-spec-gap", "status": "clear", "should_halt": False})
    write_design_artifact(workspace, ".omo/ci/spec-clear.json", sufficient=True)
    run_helper(
        script,
        ["--consumer-workspace", str(workspace), "--spec-sufficiency", ".omo/ci/spec-clear.json", "--spec-gap-status", ".omo/ci/spec-gap-clear.json", "--output", ".omo/ci/fix-clear.json", "--handoff-output", ".omo/ci/fix-clear.md"],
        {0},
    )
    clear = read_json(workspace, ".omo/ci/fix-clear.json")
    require(clear["status"] == "clear-noop" and clear["noop"] is True, "fix must distinguish clear-noop")
    require(clear["should_commit"] is False and clear["should_push"] is False, "clear-noop must not commit or push")

    spec_path = rel_path(workspace, "openspec/specs/demo.md")
    spec_path.parent.mkdir(parents=True, exist_ok=True)
    spec_path.write_text("### Requirement: In-scope deterministic defect\nThe fixture behavior is covered.\n", encoding="utf-8")
    write_review_artifact(workspace, ".omo/ci/review-sufficient.json", [finding("In-scope deterministic defect")])
    design_script = helper(actions_root, "design", "design.py")
    run_helper(
        design_script,
        ["--consumer-workspace", str(workspace), "--repository", "local-consumer", "--review-input", ".omo/ci/review-sufficient.json", "--output", ".omo/ci/spec-sufficient.json", "--plan", ".omo/ci/design-sufficient.md"],
        {0},
    )
    rel_path(workspace, "pr-touched.txt").write_text("src/lib.rs\n", encoding="utf-8")
    rel_path(workspace, "changed-fixed.txt").write_text("src/lib.rs\n", encoding="utf-8")
    run_helper(
        script,
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            ".omo/ci/spec-sufficient.json",
            "--spec-gap-status",
            ".omo/ci/spec-gap-clear.json",
            "--pr-touched",
            "pr-touched.txt",
            "--changed-files",
            "changed-fixed.txt",
            "--output",
            ".omo/ci/fix-fixed.json",
            "--handoff-output",
            ".omo/ci/fix-fixed.md",
        ],
        {0},
    )
    fixed = read_json(workspace, ".omo/ci/fix-fixed.json")
    require(fixed["status"] == "fixed" and fixed["scope_ok"] is True, "fix must distinguish fixed in-scope changes")
    require(fixed["should_commit"] is True and fixed["should_push"] is True, "fixed status must request later commit and push")

    rel_path(workspace, "changed-violation.txt").write_text("unscoped/outside.rs\n", encoding="utf-8")
    run_helper(
        script,
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            ".omo/ci/spec-sufficient.json",
            "--spec-gap-status",
            ".omo/ci/spec-gap-clear.json",
            "--pr-touched",
            "pr-touched.txt",
            "--changed-files",
            "changed-violation.txt",
            "--output",
            ".omo/ci/fix-scope-violation.json",
            "--handoff-output",
            ".omo/ci/fix-scope-violation.md",
        ],
        {1},
    )
    violation = read_json(workspace, ".omo/ci/fix-scope-violation.json")
    require(violation["status"] == "scope-violation" and violation["scope_ok"] is False, "fix must fail closed on scope violations")
    return ".omo/ci/fix-clear.json", ".omo/ci/fix-fixed.json"


def assert_verify(actions_root: Path, workspace: Path) -> str:
    script = helper(actions_root, "verify", "verify.py")
    run_helper(script, ["--consumer-workspace", str(workspace), "--fixture", "approve", "--output", ".omo/grimoire/verdict-approve.json"], {0})
    approve = read_json(workspace, ".omo/grimoire/verdict-approve.json")
    require(approve["approved"] is True, "verify approve fixture must approve")
    predicate = approve["jq_all_approve_predicate"]
    for lens in APPROVE_LENSES:
        require(f".{lens} == \"APPROVE\"" in predicate, f"verify jq predicate missing {lens}")
    run_helper(script, ["--consumer-workspace", str(workspace), "--validate", ".omo/grimoire/verdict-approve.json"], {0})

    run_helper(script, ["--consumer-workspace", str(workspace), "--fixture", "reject", "--output", ".omo/grimoire/verdict-reject.json"], {1})
    run_helper(script, ["--consumer-workspace", str(workspace), "--validate", ".omo/grimoire/verdict-reject.json"], {1})
    run_helper(script, ["--consumer-workspace", str(workspace), "--fixture", "invalid", "--output", ".omo/grimoire/verdict-invalid.json"], {1})
    run_helper(script, ["--consumer-workspace", str(workspace), "--validate", ".omo/grimoire/verdict-invalid.json"], {1})
    return ".omo/grimoire/verdict-approve.json"


def assert_cast(actions_root: Path, workspace: Path, clear_fix: str, fixed_fix: str, approve_verdict: str) -> None:
    script = helper(actions_root, "cast", "cast_driver.py")
    trusted = {
        "schema_version": 1,
        "stage": "grimoire-trusted-controller",
        "status": "ok",
        "action": "continue",
        "model_execution_allowed": True,
        "write_allowed": True,
        "commit_allowed": True,
        "push_allowed": True,
        "github_mutation_allowed": True,
    }
    write_json(workspace, ".omo/ci/trusted-controller-status.json", trusted)
    run_helper(
        script,
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
            ".omo/ci/cast-preflight.json",
        ],
        {0},
    )
    preflight = read_json(workspace, ".omo/ci/cast-preflight.json")
    require(preflight["does_not_rerun_trusted_controller"] is True, "cast preflight must consume the one trusted-controller result")

    write_design_artifact(workspace, ".omo/ci/spec-clear-for-cast.json", sufficient=True)
    write_json(workspace, ".omo/ci/issues-ok.json", {"schema_version": 1, "stage": "grimoire-cast", "status": "ok", "issue_count": 0})
    run_helper(script, ["boulder", "--consumer-workspace", str(workspace), "--fix-status", clear_fix, "--output", ".omo/boulder-clear.json"], {0})
    run_helper(
        script,
        [
            "decide",
            "--consumer-workspace",
            str(workspace),
            "--preflight-status",
            ".omo/ci/cast-preflight.json",
            "--review-status",
            ".omo/ci/review-clean.json",
            "--review-outcome",
            "success",
            "--design-status",
            ".omo/ci/spec-clear-for-cast.json",
            "--issue-status",
            ".omo/ci/issues-ok.json",
            "--fix-status",
            clear_fix,
            "--fix-outcome",
            "success",
            "--boulder-status",
            ".omo/boulder-clear.json",
            "--verdict-status",
            approve_verdict,
            "--verify-outcome",
            "success",
            "--output",
            ".omo/ci/cast-decision-clear.json",
        ],
        {0},
    )
    clear_decision = read_json(workspace, ".omo/ci/cast-decision-clear.json")
    require(clear_decision["decision"] == "clear-noop-terminal" and clear_decision["terminal"] is True, "cast must terminate clear-noop after all APPROVE")
    require(clear_decision["should_push"] is False and "no commit, no push" in clear_decision["clear_noop_terminal_semantics"], "clear-noop terminal must not commit or push")
    run_helper(script, ["complete", "--consumer-workspace", str(workspace), "--decision", ".omo/ci/cast-decision-clear.json", "--output", ".omo/ci/cast-final-clear.json"], {0})
    final_clear = read_json(workspace, ".omo/ci/cast-final-clear.json")
    require(final_clear["status"] == "terminal", "clear-noop complete status must be terminal")

    run_helper(script, ["boulder", "--consumer-workspace", str(workspace), "--fix-status", fixed_fix, "--output", ".omo/boulder-fixed.json"], {0})
    run_helper(
        script,
        [
            "decide",
            "--consumer-workspace",
            str(workspace),
            "--preflight-status",
            ".omo/ci/cast-preflight.json",
            "--review-status",
            ".omo/ci/review-clean.json",
            "--review-outcome",
            "success",
            "--design-status",
            ".omo/ci/spec-clear-for-cast.json",
            "--issue-status",
            ".omo/ci/issues-ok.json",
            "--fix-status",
            fixed_fix,
            "--fix-outcome",
            "success",
            "--boulder-status",
            ".omo/boulder-fixed.json",
            "--verdict-status",
            approve_verdict,
            "--verify-outcome",
            "success",
            "--output",
            ".omo/ci/cast-decision-fixed.json",
        ],
        {0},
    )
    fixed_decision = read_json(workspace, ".omo/ci/cast-decision-fixed.json")
    require(fixed_decision["decision"] == "scoped-push" and fixed_decision["should_push"] is True, "fixed + APPROVE must request scoped push")
    require("pull_request.synchronize" in fixed_decision["fixed_push_semantics"], "fixed push semantics must require synchronize re-review")
    write_json(workspace, ".omo/ci/cast-push-status.json", {"schema_version": 1, "stage": "grimoire-cast", "status": "pushed", "push_count": 1})
    run_helper(script, ["complete", "--consumer-workspace", str(workspace), "--decision", ".omo/ci/cast-decision-fixed.json", "--push-status", ".omo/ci/cast-push-status.json", "--output", ".omo/ci/cast-final-fixed.json"], {0})
    final_fixed = read_json(workspace, ".omo/ci/cast-final-fixed.json")
    require(final_fixed["status"] == "awaiting-synchronize" and final_fixed["next_expected_event"] == "pull_request.synchronize", "fixed completion must await synchronize re-review")

    empty_workspace = make_workspace("empty-push")
    subprocess.run(["git", "init"], cwd=str(empty_workspace), text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    write_json(empty_workspace, ".omo/ci/cast-decision.json", {"schema_version": 1, "stage": "grimoire-cast", "decision": "scoped-push", "should_push": True})
    env = os.environ.copy()
    env["GRIMOIRE_GITHUB_PAT"] = "placeholder-for-empty-commit-check"
    run_helper(
        script,
        ["push", "--consumer-workspace", str(empty_workspace), "--decision", ".omo/ci/cast-decision.json", "--repository", "local-consumer", "--consumer-ref", "grimoire-test", "--output", ".omo/ci/cast-push-status.json"],
        {1},
        env=env,
    )
    push_status = read_json(empty_workspace, ".omo/ci/cast-push-status.json")
    require(push_status["status"] == "blocked" and "empty commit" in push_status["blocked_reason"], "scoped push must refuse empty commits")


def assert_stage_contract(actions_root: Path) -> None:
    workspace = make_workspace("stage-contract")
    assert_review(actions_root, workspace)
    assert_design_and_issues(actions_root, workspace)
    assert_spec_gap(actions_root, workspace)
    clear_fix, fixed_fix = assert_fix(actions_root, workspace)
    approve_verdict = assert_verify(actions_root, workspace)
    assert_cast(actions_root, workspace, clear_fix, fixed_fix, approve_verdict)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate Grimoire stage helper semantics.")
    parser.add_argument("--actions", required=True, type=Path)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        assert_stage_contract(args.actions.resolve())
    except ContractError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print("stage contract ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
