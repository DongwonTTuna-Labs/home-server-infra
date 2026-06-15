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


def write_executable(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")
    path.chmod(0o755)


def live_review_env(workspace: Path, opencode_script: str | None) -> dict[str, str]:
    bin_dir = rel_path(workspace, "runtime-bin")
    bin_dir.mkdir(parents=True, exist_ok=True)
    python_link = bin_dir / "python3"
    if not python_link.exists():
        python_link.symlink_to(sys.executable)
    if opencode_script is not None:
        write_executable(bin_dir / "opencode", opencode_script)
    env = os.environ.copy()
    env.update(
        {
            "PATH": str(bin_dir),
            "AI_RELAY_API_KEY": "relay-fixture",
            "CF_ACCESS_CLIENT_ID": "cf-id-fixture",
            "CF_ACCESS_CLIENT_SECRET": "cf-secret-fixture",
        }
    )
    return env


def opencode_setup_env(workspace: Path, name: str, npm_script: str | None, opencode_script: str | None = None) -> dict[str, str]:
    bin_dir = rel_path(workspace, f"setup-bin-{name}")
    bin_dir.mkdir(parents=True, exist_ok=True)
    python_link = bin_dir / "python3"
    if not python_link.exists():
        python_link.symlink_to(sys.executable)
    if npm_script is not None:
        write_executable(bin_dir / "npm", npm_script)
    if opencode_script is not None:
        write_executable(bin_dir / "opencode", opencode_script)
    runner_temp = rel_path(workspace, f"runner-temp-{name}")
    runner_temp.mkdir(parents=True, exist_ok=True)
    github_output = rel_path(workspace, f".omo/ci/setup-opencode-{name}.out")
    github_path = rel_path(workspace, f".omo/ci/setup-opencode-{name}.path")
    github_output.parent.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(
        {
            "PATH": str(bin_dir) + os.pathsep + "/usr/bin" + os.pathsep + "/bin",
            "RUNNER_TEMP": str(runner_temp),
            "GITHUB_OUTPUT": str(github_output),
            "GITHUB_PATH": str(github_path),
        }
    )
    return env


def setup_output_paths(env: dict[str, str]) -> tuple[Path, Path]:
    return Path(env["GITHUB_OUTPUT"]), Path(env["GITHUB_PATH"])



def run_live_review_case(script: Path, workspace: Path, name: str, env: dict[str, str], expected: set[int]) -> tuple[subprocess.CompletedProcess[str], dict[str, Any], str]:
    output = f".omo/ci/review-{name}.json"
    github_output_path = rel_path(workspace, f".omo/ci/review-{name}.out")
    result = run_helper(script, ["--consumer-workspace", str(workspace), "--output", output, "--github-output", str(github_output_path)], expected, env=env)
    payload = read_json(workspace, output)
    github_text = github_output_path.read_text(encoding="utf-8")
    return result, payload, github_text


def assert_blocked_category(result: subprocess.CompletedProcess[str], payload: dict[str, Any], github_output: str, category: str) -> None:
    require(payload["status"] == "blocked", f"live review must block with {category}")
    require(payload.get("blocked_reason_category") == category, f"blocked payload must expose sanitized category {category}")
    require(category in result.stdout, f"review summary must expose sanitized category {category}")
    require(f"blocked_reason_category={category}" in github_output, f"GitHub output must expose sanitized category {category}")
    serialized = json.dumps(payload, sort_keys=True) + result.stdout + result.stderr + github_output
    forbidden = ("RAW_COMMAND_STDOUT_SENTINEL", "RAW_COMMAND_STDERR_SENTINEL")
    for marker in forbidden:
        require(marker not in serialized, f"blocked diagnostics leaked raw command output marker {marker}")


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


def assert_setup_opencode(actions_root: Path, workspace: Path) -> None:
    script = helper(actions_root, "cast", "setup_opencode.py")
    existing_env = opencode_setup_env(
        workspace,
        "existing",
        None,
        """#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  printf '%s\n' 'opencode 1.17.7-existing-fixture'
  exit 0
fi
exit 2
""",
    )
    existing_result = run_helper(script, [], {0}, env=existing_env)
    existing_output, existing_path = setup_output_paths(existing_env)
    require("source=existing" in existing_result.stdout, "setup-opencode must accept an already valid opencode runtime")
    require("status=ok" in existing_output.read_text(encoding="utf-8"), "setup-opencode existing path must emit ok status")
    require(not existing_path.exists() or existing_path.read_text(encoding="utf-8") == "", "setup-opencode must not rewrite PATH for existing runtime")

    installing_npm = """#!/bin/sh
prefix=''
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--prefix" ]; then
    shift
    prefix="$1"
  fi
  shift || true
done
if [ -z "$prefix" ]; then
  exit 2
fi
mkdir -p "$prefix/bin"
cat > "$prefix/bin/opencode" <<'OPENCODE_EOF'
#!/bin/sh
if [ "${1:-}" = "--version" ]; then
  printf '%s\n' 'opencode 1.17.7-installed-fixture'
  exit 0
fi
printf '%s\n' '{"status":"approved","findings":[]}'
OPENCODE_EOF
chmod 0755 "$prefix/bin/opencode"
"""
    broken_existing_env = opencode_setup_env(
        workspace,
        "broken-existing",
        installing_npm,
        """#!/bin/sh
printf '%s\n' 'BROKEN_EXISTING_OPENCODE_SENTINEL' >&2
exit 42
""",
    )
    broken_existing_result = run_helper(script, [], {0}, env=broken_existing_env)
    broken_existing_output, broken_existing_path = setup_output_paths(broken_existing_env)
    broken_existing_serialized = broken_existing_result.stdout + broken_existing_result.stderr + broken_existing_output.read_text(encoding="utf-8") + broken_existing_path.read_text(encoding="utf-8")
    require("source=npm" in broken_existing_result.stdout, "setup-opencode must replace an invalid preinstalled opencode runtime")
    require("BROKEN_EXISTING_OPENCODE_SENTINEL" not in broken_existing_serialized, "setup-opencode must not leak raw broken runtime diagnostics while replacing it")

    install_env = opencode_setup_env(workspace, "install", installing_npm)
    install_result = run_helper(script, [], {0}, env=install_env)
    install_output, install_path = setup_output_paths(install_env)
    install_output_text = install_output.read_text(encoding="utf-8")
    install_path_text = install_path.read_text(encoding="utf-8")
    require("source=npm" in install_result.stdout, "setup-opencode must provision opencode through npm when missing")
    require("package=opencode-ai" in install_output_text, "setup-opencode must identify the trusted npm package")
    require("package_version=1.17.7" in install_output_text, "setup-opencode must pin the opencode npm package version")
    require("grimoire-opencode-runtime/opencode-ai@1.17.7/bin" in install_path_text, "setup-opencode must add the provisioned opencode bin directory to later workflow steps")

    failing_npm = """#!/bin/sh
printf '%s\n' 'RAW_COMMAND_STDOUT_SENTINEL'
printf '%s\n' 'RAW_COMMAND_STDERR_SENTINEL' >&2
exit 42
"""
    failure_env = opencode_setup_env(workspace, "npm-failure", failing_npm)
    failure_result = run_helper(script, [], {1}, env=failure_env)
    failure_output, failure_path = setup_output_paths(failure_env)
    serialized = failure_result.stdout + failure_result.stderr + failure_output.read_text(encoding="utf-8")
    if failure_path.exists():
        serialized += failure_path.read_text(encoding="utf-8")
    require("runtime-failed:opencode-install-failed" in serialized, "setup-opencode must categorize npm installation failure")
    for marker in ("RAW_COMMAND_STDOUT_SENTINEL", "RAW_COMMAND_STDERR_SENTINEL"):
        require(marker not in serialized, f"setup-opencode leaked raw installer output marker {marker}")



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


def assert_live_review_diagnostics(actions_root: Path, workspace: Path) -> None:
    script = helper(actions_root, "review", "review.py")
    missing_result, missing_payload, missing_output = run_live_review_case(script, workspace, "missing-opencode", live_review_env(workspace, None), {1})
    assert_blocked_category(missing_result, missing_payload, missing_output, "missing-runtime:opencode-unavailable")

    failing_opencode = """#!/bin/sh
printf '%s\n' 'RAW_COMMAND_STDOUT_SENTINEL'
printf '%s\n' 'RAW_COMMAND_STDERR_SENTINEL' >&2
exit 42
"""
    failure_result, failure_payload, failure_output = run_live_review_case(script, workspace, "opencode-failed", live_review_env(workspace, failing_opencode), {1})
    assert_blocked_category(failure_result, failure_payload, failure_output, "runtime-failed:opencode-command-failed")

    invalid_json_opencode = """#!/usr/bin/env python3
import json
import sys
print(json.dumps({"type":"text","timestamp":1,"sessionID":"s","part":{"type":"text","text":"RAW_COMMAND_STDOUT_SENTINEL","time":{"end":1}}}))
print("RAW_COMMAND_STDERR_SENTINEL", file=sys.stderr)
"""
    invalid_result, invalid_payload, invalid_output = run_live_review_case(script, workspace, "invalid-json", live_review_env(workspace, invalid_json_opencode), {1})
    assert_blocked_category(invalid_result, invalid_payload, invalid_output, "contract-invalid:review-json-invalid")

    jsonl_approved_opencode = """#!/usr/bin/env python3
import json
print(json.dumps({"type":"step_start","timestamp":1,"sessionID":"s","part":{"type":"step-start"}}))
print(json.dumps({"type":"text","timestamp":2,"sessionID":"s","part":{"type":"text","text":json.dumps({"status":"approved","findings":[]}),"time":{"end":2}}}))
print(json.dumps({"type":"step_finish","timestamp":3,"sessionID":"s","part":{"type":"step-finish"}}))
"""
    jsonl_approved_result, jsonl_approved_payload, jsonl_approved_output = run_live_review_case(script, workspace, "jsonl-approved", live_review_env(workspace, jsonl_approved_opencode), {0})
    require(jsonl_approved_payload["status"] == "approved", "OpenCode JSONL text event with empty findings must approve")
    require(jsonl_approved_payload["findings_count"] == 0, "OpenCode JSONL approved event must keep zero findings")
    require("status=approved findings=0" in jsonl_approved_result.stdout, "OpenCode JSONL approved summary must show approval")
    require("status=approved" in jsonl_approved_output, "OpenCode JSONL approved GitHub output must expose approved status")

    jsonl_findings_opencode = """#!/usr/bin/env python3
import json
finding = {
  "file":"src/lib.rs",
  "line":7,
  "severity":"medium",
  "lens":"correctness",
  "title":"OpenCode JSONL finding",
  "what":"The OpenCode JSONL fixture emitted a review finding.",
  "why":"The parser must normalize assistant text events into Grimoire review artifacts.",
  "suggested_fix":"Parse the final assistant text event conservatively.",
  "evidence":"src/lib.rs:7 deterministic JSONL fixture evidence"
}
print(json.dumps({"type":"text","timestamp":2,"sessionID":"s","part":{"type":"text","text":json.dumps({"status":"findings","findings":[finding]}),"time":{"end":2}}}))
"""
    jsonl_findings_result, jsonl_findings_payload, jsonl_findings_output = run_live_review_case(script, workspace, "jsonl-findings", live_review_env(workspace, jsonl_findings_opencode), {0})
    require(jsonl_findings_payload["status"] == "findings", "OpenCode JSONL text event with findings must emit findings status")
    require(jsonl_findings_payload["findings_count"] == 1, "OpenCode JSONL findings event must preserve one finding")
    require("status=findings findings=1" in jsonl_findings_result.stdout, "OpenCode JSONL findings summary must show one finding")
    require("findings_count=1" in jsonl_findings_output, "OpenCode JSONL findings GitHub output must expose finding count")

    ambiguous_jsonl_opencode = """#!/usr/bin/env python3
import json
finding = {
  "file":"src/lib.rs",
  "line":7,
  "severity":"medium",
  "lens":"correctness",
  "title":"Ambiguous JSONL finding",
  "what":"The fixture emitted a second distinct review payload.",
  "why":"Multiple distinct review payloads must fail closed.",
  "suggested_fix":"Emit exactly one final review JSON payload.",
  "evidence":"src/lib.rs:7 deterministic ambiguous fixture evidence"
}
print(json.dumps({"type":"text","timestamp":1,"sessionID":"s","part":{"type":"text","text":json.dumps({"status":"approved","findings":[]}),"time":{"end":1}}}))
print(json.dumps({"type":"text","timestamp":2,"sessionID":"s","part":{"type":"text","text":json.dumps({"status":"findings","findings":[finding]}),"time":{"end":2}}}))
"""
    ambiguous_result, ambiguous_payload, ambiguous_output = run_live_review_case(script, workspace, "jsonl-ambiguous", live_review_env(workspace, ambiguous_jsonl_opencode), {1})
    assert_blocked_category(ambiguous_result, ambiguous_payload, ambiguous_output, "contract-invalid:review-json-invalid")

    approved_opencode = """#!/bin/sh
python3 - "$OPENCODE_CONFIG" <<'PY'
import json
import pathlib
import sys
payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
if "grimoire_policy" in payload:
    print("RAW_COMMAND_STDOUT_SENTINEL")
    print("RAW_COMMAND_STDERR_SENTINEL", file=sys.stderr)
    raise SystemExit(42)
PY
printf '%s\n' '{"status":"approved","findings":[]}'
"""
    approved_result, approved_payload, approved_output = run_live_review_case(script, workspace, "approved-empty", live_review_env(workspace, approved_opencode), {0})
    require(approved_payload["status"] == "approved", "live empty findings must approve")
    require(approved_payload["findings_count"] == 0, "live empty findings must keep zero finding count")
    require("blocked_reason_category" not in approved_payload, "approved live review must not emit a blocked category")
    require("status=approved findings=0" in approved_result.stdout, "approved summary must show empty-finding approval")
    require("status=approved" in approved_output, "GitHub output must expose approved status")


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

    smoke_spec = rel_path(workspace, "openspec/changes/grimoire-push-smoke/specs/grimoire-push-smoke/spec.md")
    smoke_spec.parent.mkdir(parents=True, exist_ok=True)
    smoke_spec.write_text(
        """## ADDED Requirements

### Requirement: Grimoire push smoke marker
The repository SHALL contain `docs/GRIMOIRE_PUSH_SMOKE.md` with the exact canonical Markdown content specified in `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.

#### Scenario: Grimoire recreates the missing marker
- **THEN** Grimoire creates exactly `docs/GRIMOIRE_PUSH_SMOKE.md` with the canonical content from `docs/GRIMOIRE_PUSH_SMOKE.spec.md`
""",
        encoding="utf-8",
    )
    write_review_artifact(workspace, ".omo/ci/review-smoke.json", [finding("Missing Grimoire push smoke marker", "docs/GRIMOIRE_PUSH_SMOKE.md")])
    run_helper(
        design_script,
        ["--consumer-workspace", str(workspace), "--repository", "local-consumer", "--review-input", ".omo/ci/review-smoke.json", "--output", ".omo/ci/spec-smoke.json", "--plan", ".omo/ci/design-smoke.md"],
        {0},
    )
    smoke_design = read_json(workspace, ".omo/ci/spec-smoke.json")
    require("docs/GRIMOIRE_PUSH_SMOKE.md" in smoke_design.get("allowed_write_paths", []), "design must emit spec-bound allowed write paths for the docs marker")
    rel_path(workspace, "changed-smoke-marker.txt").write_text("docs/GRIMOIRE_PUSH_SMOKE.md\n", encoding="utf-8")
    run_helper(
        script,
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            ".omo/ci/spec-smoke.json",
            "--spec-gap-status",
            ".omo/ci/spec-gap-clear.json",
            "--pr-touched",
            "pr-touched.txt",
            "--changed-files",
            "changed-smoke-marker.txt",
            "--output",
            ".omo/ci/fix-smoke-marker.json",
            "--handoff-output",
            ".omo/ci/fix-smoke-marker.md",
        ],
        {0},
    )
    smoke_fix = read_json(workspace, ".omo/ci/fix-smoke-marker.json")
    require(smoke_fix["status"] == "fixed" and smoke_fix["scope_ok"] is True, "fix must allow a spec-bound docs marker target")
    require("docs/GRIMOIRE_PUSH_SMOKE.md" in smoke_fix["allowed_paths"], "fix allowed paths must record the spec-bound docs marker")
    require(smoke_fix["allowed_write_paths"] == ["docs/GRIMOIRE_PUSH_SMOKE.md"], "fix must expose normalized allowed write paths")

    rel_path(workspace, "changed-smoke-source.txt").write_text("src/unscoped.rs\n", encoding="utf-8")
    run_helper(
        script,
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            ".omo/ci/spec-smoke.json",
            "--spec-gap-status",
            ".omo/ci/spec-gap-clear.json",
            "--pr-touched",
            "pr-touched.txt",
            "--changed-files",
            "changed-smoke-source.txt",
            "--output",
            ".omo/ci/fix-smoke-source-violation.json",
            "--handoff-output",
            ".omo/ci/fix-smoke-source-violation.md",
        ],
        {1},
    )
    smoke_source_violation = read_json(workspace, ".omo/ci/fix-smoke-source-violation.json")
    require(smoke_source_violation["status"] == "scope-violation" and "src/unscoped.rs" in smoke_source_violation["violations"], "spec-bound docs target must not allow arbitrary source edits")

    write_design_artifact(workspace, ".omo/ci/spec-traversal-target.json", sufficient=True, in_scope=[{"path": "docs/authorized.md"}], bindings=[{"target_paths": ["../escape.md", "docs/authorized.md"]}])
    rel_path(workspace, "changed-authorized-doc.txt").write_text("docs/authorized.md\n", encoding="utf-8")
    run_helper(
        script,
        [
            "--consumer-workspace",
            str(workspace),
            "--spec-sufficiency",
            ".omo/ci/spec-traversal-target.json",
            "--spec-gap-status",
            ".omo/ci/spec-gap-clear.json",
            "--changed-files",
            "changed-authorized-doc.txt",
            "--output",
            ".omo/ci/fix-traversal-target.json",
            "--handoff-output",
            ".omo/ci/fix-traversal-target.md",
        ],
        {1},
    )
    traversal_target = read_json(workspace, ".omo/ci/fix-traversal-target.json")
    require(traversal_target["status"] == "scope-violation" and traversal_target["invalid_spec_target_paths"] == ["../escape.md"], "fix must fail closed on traversal in spec target metadata")

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

    verify_workspace = make_workspace("verify-live")
    subprocess.run(["git", "init"], cwd=str(verify_workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    marker_spec = rel_path(verify_workspace, "docs/GRIMOIRE_PUSH_SMOKE.spec.md")
    marker_spec.parent.mkdir(parents=True, exist_ok=True)
    marker_spec.write_text(
        """# Grimoire Push Smoke Spec

## Canonical Markdown

```markdown
# Grimoire Push Smoke

This file documents the Grimoire reusable control-plane push smoke for the OpenSpec-backed `grimoire-push-smoke` change.
```
""",
        encoding="utf-8",
    )
    subprocess.run(["git", "config", "user.name", "grimoire-verify-test"], cwd=str(verify_workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    subprocess.run(["git", "config", "user.email", "grimoire-verify-test@dongwontuna-labs.invalid"], cwd=str(verify_workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    subprocess.run(["git", "add", "docs/GRIMOIRE_PUSH_SMOKE.spec.md"], cwd=str(verify_workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    subprocess.run(["git", "commit", "-m", "test: add marker spec baseline"], cwd=str(verify_workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    marker = rel_path(verify_workspace, "docs/GRIMOIRE_PUSH_SMOKE.md")
    marker.write_text(
        "# Grimoire Push Smoke\n\n"
        "This file documents the Grimoire reusable control-plane push smoke for the OpenSpec-backed `grimoire-push-smoke` change.\n",
        encoding="utf-8",
    )
    write_design_artifact(verify_workspace, ".omo/ci/spec-verify-live.json", sufficient=True, in_scope=[{"path": "docs/GRIMOIRE_PUSH_SMOKE.md"}], bindings=[{"allowed_write_paths": ["docs/GRIMOIRE_PUSH_SMOKE.md"]}])
    write_json(verify_workspace, ".omo/ci/spec-gap-clear-verify.json", {"schema_version": 1, "stage": "grimoire-spec-gap", "status": "clear", "should_halt": False})
    write_json(
        verify_workspace,
        ".omo/ci/fix-verify-live.json",
        {
            "schema_version": 1,
            "stage": "grimoire-fix",
            "status": "fixed",
            "scope_ok": True,
            "changed_files": ["docs/GRIMOIRE_PUSH_SMOKE.md"],
            "allowed_paths": ["docs/GRIMOIRE_PUSH_SMOKE.md"],
            "allowed_write_paths": ["docs/GRIMOIRE_PUSH_SMOKE.md"],
            "violations": [],
        },
    )
    run_helper(
        script,
        ["--consumer-workspace", str(verify_workspace), "--spec-sufficiency", ".omo/ci/spec-verify-live.json", "--spec-gap-status", ".omo/ci/spec-gap-clear-verify.json", "--fix-status", ".omo/ci/fix-verify-live.json", "--output", ".omo/grimoire/verdict-live-approve.json"],
        {0},
    )
    live_approve = read_json(verify_workspace, ".omo/grimoire/verdict-live-approve.json")
    require(live_approve["approved"] is True and live_approve["f4_scope"] == "APPROVE", "verify live path must approve the authorized marker-only fix")

    marker.write_text("# Grimoire Push Smoke\n\nUnexpected modified content.\n", encoding="utf-8")
    run_helper(
        script,
        ["--consumer-workspace", str(verify_workspace), "--spec-sufficiency", ".omo/ci/spec-verify-live.json", "--spec-gap-status", ".omo/ci/spec-gap-clear-verify.json", "--fix-status", ".omo/ci/fix-verify-live.json", "--output", ".omo/grimoire/verdict-live-reject.json"],
        {1},
    )
    live_reject = read_json(verify_workspace, ".omo/grimoire/verdict-live-reject.json")
    require(live_reject["approved"] is False and live_reject["f2_quality"] == "REJECT", "verify live path must reject marker content drift")
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
    assert_setup_opencode(actions_root, workspace)
    assert_review(actions_root, workspace)
    assert_live_review_diagnostics(actions_root, workspace)
    assert_design_and_issues(actions_root, workspace)
    assert_spec_gap(actions_root, workspace)
    clear_fix, fixed_fix = assert_fix(actions_root, workspace)
    approve_verdict = assert_verify(actions_root, workspace)
    assert_cast(actions_root, workspace, clear_fix, fixed_fix, approve_verdict)


def test_grimoire_stage_contract() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    assert_stage_contract(repo_root / "actions" / "grimoire")


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
