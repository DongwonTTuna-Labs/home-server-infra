# pyright: reportAny=false, reportExplicitAny=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shutil
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


def load_script_module(path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(path.stem, path)
    if spec is None or spec.loader is None:
        raise ContractError(f"unable to load script module: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


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


def finding(title: str, path: str = "src/lib.rs", *, severity: str = "medium", out_of_scope: bool = False, target_paths: list[str] | None = None) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "file": path,
        "line": 7,
        "severity": severity,
        "lens": "correctness",
        "title": title,
        "what": f"{title} was observed in the deterministic contract fixture.",
        "why": "The contract test needs a stable stage artifact to validate loop semantics.",
        "suggested_fix": "Bind the finding to OpenSpec or track it outside the loop.",
        "evidence": f"{path}:7 deterministic fixture evidence",
    }
    if target_paths is not None:
        payload["target_paths"] = target_paths
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


def run_design_artifact(actions_root: Path, workspace: Path, review_input: str, output: str, plan: str, expected: set[int]) -> dict[str, Any]:
    script = helper(actions_root, "design", "design.py")
    run_helper(
        script,
        ["--consumer-workspace", str(workspace), "--repository", "local-consumer", "--review-input", review_input, "--output", output, "--plan", plan],
        expected,
    )
    return read_json(workspace, output)


def write_scope_manifest(workspace: Path, content: str) -> None:
    path = rel_path(workspace, ".omo/grimoire/scope.yml")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def remove_scope_manifest(workspace: Path) -> None:
    path = rel_path(workspace, ".omo/grimoire/scope.yml")
    if path.exists():
        path.unlink()


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

    target_prompt_opencode = """#!/usr/bin/env python3
import json
import sys
prompt = sys.argv[-1]
if "target_paths" not in prompt or "file to create or modify" not in prompt:
    raise SystemExit(42)
finding = {
  "file":"docs/GRIMOIRE_PUSH_SMOKE.spec.md",
  "line":1,
  "severity":"medium",
  "lens":"correctness",
  "title":"Required smoke marker is missing",
  "what":"The required marker file is absent.",
  "why":"The smoke fixture requires the marker file to exist.",
  "suggested_fix":"Create the required marker file from the spec.",
  "evidence":"docs/GRIMOIRE_PUSH_SMOKE.spec.md:1 names the required marker.",
  "target_paths":["docs/GRIMOIRE_PUSH_SMOKE.md"]
}
print(json.dumps({"status":"findings","findings":[finding]}))
"""
    target_result, target_payload, _ = run_live_review_case(script, workspace, "target-path", live_review_env(workspace, target_prompt_opencode), {0})
    target_finding = target_payload["findings"][0]
    require(target_finding["target_paths"] == ["docs/GRIMOIRE_PUSH_SMOKE.md"], "live review must preserve explicit finding target_paths")
    require("status=findings findings=1" in target_result.stdout, "target-path review summary must show one finding")


def assert_design_and_issues(actions_root: Path, workspace: Path) -> None:
    cast_script = helper(actions_root, "cast", "cast_driver.py")
    remove_scope_manifest(workspace)
    write_review_artifact(
        workspace,
        ".omo/ci/review-design-advisory.json",
        [
            finding("Info severity missing spec", "src/info.rs", severity="info"),
            finding("Low severity missing spec", "src/low.rs", severity="low"),
            finding("Medium severity missing spec", "src/lib.rs", severity="medium"),
            finding("[out-of-scope] Separate infra cleanup", "infra/cleanup.md", out_of_scope=True),
        ],
    )
    advisory = run_design_artifact(actions_root, workspace, ".omo/ci/review-design-advisory.json", ".omo/ci/spec-advisory.json", ".omo/ci/design-advisory.md", {0})
    require(advisory["status"] == "sufficient" and advisory["should_halt"] is False, "medium/low/info missing OpenSpec evidence must be advisory")
    require(advisory["gate_mode"] == "severity-threshold" and advisory["manifest_status"] == "absent", "absent scope manifest must use severity-threshold gate")
    require(advisory["missing"] == [] and advisory["safety_default_gaps"] == [], "advisory-only severity gaps must not populate hard halt arrays")
    require({item["severity"] for item in advisory["advisory_gaps"]} == {"info", "low", "medium"}, "info/low/medium missing spec gaps must be advisory")
    require(len(advisory["in_scope"]) == 3 and len(advisory["out_of_scope"]) == 1, "design must split advisory in_scope and out_of_scope findings")
    first_fingerprint = advisory["out_of_scope"][0]["fingerprint"]
    require(re.fullmatch(r"[0-9a-f]{24}", first_fingerprint) is not None, "out-of-scope fingerprint must be stable 24-char hex")
    repeated = run_design_artifact(actions_root, workspace, ".omo/ci/review-design-advisory.json", ".omo/ci/spec-advisory-repeat.json", ".omo/ci/design-repeat.md", {0})
    require(repeated["out_of_scope"][0]["fingerprint"] == first_fingerprint, "out-of-scope fingerprint must be stable across runs")

    run_helper(
        cast_script,
        [
            "file-issues",
            "--consumer-workspace",
            str(workspace),
            "--design-path",
            ".omo/ci/spec-advisory.json",
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
            ".omo/ci/spec-advisory.json",
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

    write_review_artifact(
        workspace,
        ".omo/ci/review-hard-halt.json",
        [finding("High severity missing spec", "src/high.rs", severity="high"), finding("Critical severity missing spec", "src/critical.rs", severity="critical")],
    )
    hard_halt = run_design_artifact(actions_root, workspace, ".omo/ci/review-hard-halt.json", ".omo/ci/spec-insufficient.json", ".omo/ci/design-insufficient.md", {1})
    require(hard_halt["status"] == "insufficient" and hard_halt["should_halt"] is True, "high/critical missing OpenSpec evidence must halt")
    require({item["severity"] for item in hard_halt["missing"]} == {"high", "critical"}, "hard halt missing entries must retain high/critical severities")
    require(len(hard_halt["safety_default_gaps"]) == 2, "high/critical missing evidence must remain fail-closed safety gaps")
    require("High severity missing spec" in hard_halt["suggested_spec_patch"] and "Critical severity missing spec" in hard_halt["suggested_spec_patch"], "hard halt must include suggested spec patch text")

    write_scope_manifest(workspace, "version: 1\ngoverned_paths:\n  - src/governed/**\nadvisory_only_paths:\n  - docs/advisory/**\n")
    write_review_artifact(workspace, ".omo/ci/review-governed-high.json", [finding("Governed high missing spec", "src/governed/main.rs", severity="high")])
    governed = run_design_artifact(actions_root, workspace, ".omo/ci/review-governed-high.json", ".omo/ci/spec-governed-high.json", ".omo/ci/design-governed-high.md", {1})
    require(governed["gate_mode"] == "scope-manifest" and governed["manifest_status"] == "loaded", "valid scope manifest must activate scope-manifest gate")
    require(governed["should_halt"] is True and governed["in_scope"][0]["scope_manifest_classification"] == "governed", "governed high finding must halt")

    write_review_artifact(
        workspace,
        ".omo/ci/review-ungoverned-advisory.json",
        [finding("Ungoverned high finding", "src/elsewhere/main.rs", severity="high"), finding("Advisory-only high finding", "docs/advisory/note.md", severity="high")],
    )
    ungoverned = run_design_artifact(actions_root, workspace, ".omo/ci/review-ungoverned-advisory.json", ".omo/ci/spec-ungoverned-advisory.json", ".omo/ci/design-ungoverned-advisory.md", {0})
    reasons = {item["out_of_scope_reason"] for item in ungoverned["out_of_scope"]}
    require(ungoverned["status"] == "sufficient" and ungoverned["missing"] == [], "ungoverned/advisory-only scope manifest findings must not halt")
    require(reasons == {"scope-manifest-ungoverned", "scope-manifest-advisory-only"}, "scope manifest must retain ungoverned/advisory-only issue reasons")

    write_scope_manifest(workspace, "version: 1\ngoverned_paths: [\n")
    malformed = run_design_artifact(actions_root, workspace, ".omo/ci/review-governed-high.json", ".omo/ci/spec-malformed-manifest.json", ".omo/ci/design-malformed-manifest.md", {1})
    require(malformed["gate_mode"] == "severity-threshold" and malformed["manifest_status"] == "malformed", "malformed scope manifest must fall back to severity-threshold")
    remove_scope_manifest(workspace)
    absent = run_design_artifact(actions_root, workspace, ".omo/ci/review-governed-high.json", ".omo/ci/spec-absent-manifest.json", ".omo/ci/design-absent-manifest.md", {1})
    require(absent["gate_mode"] == "severity-threshold" and absent["manifest_status"] == "absent", "absent scope manifest must fall back to severity-threshold")

    write_scope_manifest(workspace, "version: 1\ngoverned_paths:\n  - src/unsafe/**\n")
    write_review_artifact(workspace, ".omo/ci/review-unsafe-target.json", [finding("Unsafe target path", "src/unsafe/main.rs", severity="low", target_paths=["../escape.md"])])
    unsafe = run_design_artifact(actions_root, workspace, ".omo/ci/review-unsafe-target.json", ".omo/ci/spec-unsafe-target.json", ".omo/ci/design-unsafe-target.md", {1})
    require(unsafe["invalid_allowed_write_paths"] == ["../escape.md"] and unsafe["should_halt"] is True, "unsafe target paths must fail closed even below severity threshold")
    remove_scope_manifest(workspace)


def assert_spec_gap(actions_root: Path, workspace: Path) -> None:
    script = helper(actions_root, "spec-gap", "spec_gap.py")
    run_helper(
        script,
        ["--consumer-workspace", str(workspace), "--input", ".omo/ci/spec-insufficient.json", "--comment-output", ".omo/ci/spec-gap-comment.md", "--status-output", ".omo/ci/spec-gap-status.json"],
        {0},
    )
    status = read_json(workspace, ".omo/ci/spec-gap-status.json")
    require(status["status"] == "halt" and status["should_halt"] is True and status["should_comment"] is True, "spec-gap must emit halt status")
    require(status["advisory"] is True, "spec-gap halt status must be advisory for neutral completion")
    require(status["no_code_or_push_action"] is True, "spec-gap advisory must continue forbidding code or push action")
    require(status["top_level_sections"] == SPEC_GAP_SECTIONS and status["top_level_section_count"] == 5, "spec-gap must expose exactly five top-level sections")
    comment = rel_path(workspace, ".omo/ci/spec-gap-comment.md").read_text(encoding="utf-8")
    require(comment.splitlines()[0] == "<!-- grimoire-spec-gap -->", "spec-gap comment marker must remain the first line")
    headings = [line.removeprefix("## ") for line in comment.splitlines() if line.startswith("## ")]
    require(headings == SPEC_GAP_SECTIONS, "spec-gap comment headings drifted")
    require(comment.count("<!-- grimoire-spec-gap -->") == 1, "spec-gap comment must retain exactly one idempotent marker")
    require("High severity missing spec" in comment and "Critical severity missing spec" in comment, "spec-gap comment must retain suggested patch content")
    require("src/high.rs:7" in comment and "src/critical.rs:7" in comment, "spec-gap comment must name concrete missing evidence locations")
    require("src/high.rs" in comment and "src/critical.rs" in comment, "spec-gap comment must name concrete affected paths")
    require("### Requirement:" in comment and "#### Scenario:" in comment, "spec-gap comment must include copy-pasteable OpenSpec skeleton headings")
    require("What To Modify vs Add" in comment and "Modify existing OpenSpec evidence" in comment and "Add a new OpenSpec requirement/scenario" in comment, "spec-gap comment must distinguish modify vs add guidance")
    require("How To Clear" in comment and "📋 Spec Needed" in comment and "pull_request.synchronize" in comment, "spec-gap comment must explain label and synchronize rerun clearing")
    require("advisory/non-blocking guidance" in comment and "not a hard red failure" in comment, "spec-gap comment must frame rerun guidance as non-blocking")


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
    write_review_artifact(workspace, ".omo/ci/review-smoke.json", [finding("Missing Grimoire push smoke marker", "docs/GRIMOIRE_PUSH_SMOKE.spec.md", target_paths=["docs/GRIMOIRE_PUSH_SMOKE.md"])])
    run_helper(
        design_script,
        ["--consumer-workspace", str(workspace), "--repository", "local-consumer", "--review-input", ".omo/ci/review-smoke.json", "--output", ".omo/ci/spec-smoke.json", "--plan", ".omo/ci/design-smoke.md"],
        {0},
    )
    smoke_design = read_json(workspace, ".omo/ci/spec-smoke.json")
    require("docs/GRIMOIRE_PUSH_SMOKE.md" in smoke_design.get("allowed_write_paths", []), "design must emit explicit finding target_paths as allowed write paths")
    require("docs/GRIMOIRE_PUSH_SMOKE.spec.md" not in smoke_design.get("allowed_write_paths", []), "design must not conflate evidence file with explicit write target")
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

    write_review_artifact(workspace, ".omo/ci/review-invalid-target.json", [finding("Invalid target path", "docs/evidence.md", target_paths=["../escape.md"])])
    run_helper(
        design_script,
        ["--consumer-workspace", str(workspace), "--repository", "local-consumer", "--review-input", ".omo/ci/review-invalid-target.json", "--output", ".omo/ci/spec-invalid-target.json", "--plan", ".omo/ci/design-invalid-target.md"],
        {1},
    )
    invalid_target_design = read_json(workspace, ".omo/ci/spec-invalid-target.json")
    require(invalid_target_design["status"] == "insufficient" and "../escape.md" in invalid_target_design.get("invalid_allowed_write_paths", []), "design must fail closed on traversal finding target paths")

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
        "# Grimoire Push Smoke\n\nThis file documents the Grimoire reusable control-plane push smoke for the OpenSpec-backed `grimoire-push-smoke` change.\n",
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


def load_decide_fixture_module(actions_root: Path) -> Any:
    path = actions_root.parents[1] / "tests" / "fixtures" / "grimoire" / "run-loop-fixtures.py"
    spec = importlib.util.spec_from_file_location("grimoire_run_loop_fixtures", path)
    if spec is None or spec.loader is None:
        raise ContractError(f"unable to load decide fixture helper: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def copy_decide_workspace(root: Path, case: dict[str, Any]) -> Path:
    source = Path(str(case["fixture_dir"]))
    require(source.is_dir(), f"missing decide fixture directory: {source}")
    workspace = root / str(case["name"])
    if workspace.exists():
        shutil.rmtree(workspace)
    shutil.copytree(source, workspace)
    return workspace


def complete_status_for(decision: dict[str, Any]) -> tuple[int, str, str]:
    conclusion = str(decision.get("conclusion"))
    if conclusion == "neutral":
        return 0, "advisory", "neutral"
    if conclusion == "failure":
        return 1, "fizzled", "failure"
    if decision.get("decision") == "scoped-push":
        return 0, "awaiting-synchronize", "success"
    return 0, "terminal", "success"


def run_complete_case(script: Path, workspace: Path, decision_path: str, output: str, expected_exit: int, expected_status: str, expected_conclusion: str, push_status: str | None = None) -> dict[str, Any]:
    args = ["complete", "--consumer-workspace", str(workspace), "--decision", decision_path, "--output", output]
    if push_status is not None:
        args.extend(["--push-status", push_status])
    run_helper(script, args, {expected_exit})
    final = read_json(workspace, output)
    require(final["status"] == expected_status, f"complete status mismatch for {decision_path}: {final}")
    require(final["conclusion"] == expected_conclusion, f"complete conclusion mismatch for {decision_path}: {final}")
    require(isinstance(final.get("summary"), str) and final["summary"], "complete must emit a non-empty summary")
    return final


def write_spec_gap_upsert_inputs(workspace: Path, decision: str, gap: str, comment: str, body: str) -> None:
    write_json(
        workspace,
        decision,
        {
            "schema_version": 1,
            "stage": "grimoire-cast",
            "status": "ok",
            "decision": "spec-gap-halt",
            "conclusion": "neutral",
            "label_transition": "spec-needed",
            "exit_code": 0,
            "terminal": False,
            "should_push": False,
            "reasons": ["deterministic spec gap"],
        },
    )
    write_json(
        workspace,
        gap,
        {
            "schema_version": 1,
            "stage": "grimoire-spec-gap",
            "status": "halt",
            "should_halt": True,
            "should_comment": True,
            "advisory": True,
            "no_code_or_push_action": True,
            "comment_path": comment,
        },
    )
    rel_path(workspace, comment).parent.mkdir(parents=True, exist_ok=True)
    rel_path(workspace, comment).write_text(body, encoding="utf-8")


def run_upsert_module(module: Any, workspace: Path, output: str, *, allowed: str = "true", decision: str = ".omo/ci/upsert-decision.json", gap: str = ".omo/ci/upsert-gap.json", comment: str = ".omo/ci/upsert-comment.md") -> int:
    return int(
        module.main(
            [
                "upsert-spec-gap-comment",
                "--consumer-workspace",
                str(workspace),
                "--decision",
                decision,
                "--spec-gap-status",
                gap,
                "--comment-path",
                comment,
                "--repository",
                "DongwonTTuna-Labs/example",
                "--pr-number",
                "17",
                "--github-mutation-allowed",
                allowed,
                "--output",
                output,
            ]
        )
    )


def assert_spec_gap_comment_upsert(actions_root: Path, workspace: Path) -> None:
    module = load_script_module(helper(actions_root, "cast", "cast_driver.py"))
    marker = str(module.SPEC_GAP_COMMENT_MARKER)
    comment_body = marker + "\n\n## Summary\nInitial deterministic advisory.\n"
    updated_body = marker + "\n\n## Summary\nUpdated deterministic advisory.\n"
    write_spec_gap_upsert_inputs(workspace, ".omo/ci/upsert-decision.json", ".omo/ci/upsert-gap.json", ".omo/ci/upsert-comment.md", comment_body)
    comments: list[dict[str, Any]] = [{"id": 10, "body": "ordinary reviewer comment", "html_url": "https://example.invalid/comment/10"}]
    calls: list[tuple[str, str, dict[str, Any] | None]] = []
    next_comment_id = 40
    original_request = module.github_request
    original_token = os.environ.get("GRIMOIRE_GITHUB_PAT")

    def fake_github_request(method: str, path: str, token: str, payload: dict[str, Any] | None = None) -> Any:
        nonlocal next_comment_id
        require(token == "fixture-token", "spec-gap upsert must use GRIMOIRE_GITHUB_PAT without exposing it")
        calls.append((method, path, payload))
        if method == "GET":
            require(path.startswith("/repos/DongwonTTuna-Labs/example/issues/17/comments?per_page=100&page="), "upsert must list issue comments with pagination")
            return [dict(comment) for comment in comments]
        if method == "POST":
            require(path == "/repos/DongwonTTuna-Labs/example/issues/17/comments", "upsert must create comments through the issue comments endpoint")
            if payload is None or payload.get("body") != comment_body:
                raise ContractError("POST must carry the rendered spec-gap comment")
            next_comment_id += 1
            created = {"id": next_comment_id, "body": str(payload["body"]), "html_url": f"https://example.invalid/comment/{next_comment_id}"}
            comments.append(created)
            return dict(created)
        if method == "PATCH":
            require(path == f"/repos/DongwonTTuna-Labs/example/issues/comments/{next_comment_id}", "upsert must patch the existing marker comment")
            if payload is None or payload.get("body") != updated_body:
                raise ContractError("PATCH must carry the updated spec-gap comment")
            for comment in comments:
                if comment["id"] == next_comment_id:
                    comment["body"] = str(payload["body"])
                    return dict(comment)
        raise ContractError(f"unexpected fake GitHub request: {method} {path}")

    module.github_request = fake_github_request
    os.environ["GRIMOIRE_GITHUB_PAT"] = "fixture-token"
    try:
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-created.json") == 0, "first spec-gap upsert must create a comment")
        created = read_json(workspace, ".omo/ci/upsert-created.json")
        require(created["status"] == "ok" and created["operation"] == "created", "first upsert must report created")
        require(len([comment for comment in comments if marker in comment["body"]]) == 1, "first upsert must create exactly one marker comment")
        require([call[0] for call in calls] == ["GET", "POST"], "first upsert must GET then POST the marker comment")

        rel_path(workspace, ".omo/ci/upsert-comment.md").write_text(updated_body, encoding="utf-8")
        calls.clear()
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-patched.json") == 0, "second spec-gap upsert must patch the existing comment")
        patched = read_json(workspace, ".omo/ci/upsert-patched.json")
        marker_comments = [comment for comment in comments if marker in comment["body"]]
        require(patched["status"] == "ok" and patched["operation"] == "patched", "second upsert must report patched")
        require(len(marker_comments) == 1 and marker_comments[0]["body"] == updated_body, "second upsert must update without duplicating marker comments")
        require([call[0] for call in calls] == ["GET", "PATCH"], "second upsert must GET then PATCH the marker comment")

        calls.clear()
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-skipped.json", allowed="false") == 0, "mutation-disallowed upsert must skip cleanly")
        skipped = read_json(workspace, ".omo/ci/upsert-skipped.json")
        require(skipped["status"] == "skipped" and skipped["comment_mutation_attempted"] is False and calls == [], "mutation-disallowed upsert must not touch GitHub")

        write_json(workspace, ".omo/ci/upsert-failure-decision.json", {"schema_version": 1, "stage": "grimoire-cast", "status": "fizzled", "decision": "fizzled", "conclusion": "failure", "exit_code": 1})
        calls.clear()
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-failure-blocked.json", decision=".omo/ci/upsert-failure-decision.json") == 1, "failure decisions must not post advisory comments")
        failure = read_json(workspace, ".omo/ci/upsert-failure-blocked.json")
        require(failure["status"] == "blocked" and failure["comment_mutation_attempted"] is False and calls == [], "failure path must block before GitHub mutation")

        write_json(workspace, ".omo/ci/upsert-no-actionable-decision.json", {"schema_version": 1, "stage": "grimoire-cast", "status": "ok", "decision": "no-actionable-work", "conclusion": "neutral", "label_transition": "spec-needed", "exit_code": 0})
        calls.clear()
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-no-actionable-blocked.json", decision=".omo/ci/upsert-no-actionable-decision.json") == 1, "non-spec-gap neutral decisions must not post advisory comments")
        no_actionable = read_json(workspace, ".omo/ci/upsert-no-actionable-blocked.json")
        require(no_actionable["status"] == "blocked" and no_actionable["comment_mutation_attempted"] is False and calls == [], "non-spec-gap neutral path must block before GitHub mutation")

        write_json(workspace, ".omo/ci/upsert-malformed-gap.json", {"schema_version": 1, "stage": "grimoire-spec-gap", "status": "clear", "should_halt": False, "should_comment": False})
        calls.clear()
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-malformed-gap-blocked.json", gap=".omo/ci/upsert-malformed-gap.json") == 1, "malformed spec-gap statuses must not post advisory comments")
        malformed = read_json(workspace, ".omo/ci/upsert-malformed-gap-blocked.json")
        require(malformed["status"] == "blocked" and malformed["comment_mutation_attempted"] is False and calls == [], "malformed spec-gap status must block before GitHub mutation")

        rel_path(workspace, ".omo/ci/upsert-comment.md").unlink()
        calls.clear()
        require(run_upsert_module(module, workspace, ".omo/ci/upsert-missing-comment.json") == 1, "missing comment files must block")
        missing = read_json(workspace, ".omo/ci/upsert-missing-comment.json")
        require(missing["status"] == "blocked" and missing["comment_mutation_attempted"] is False and calls == [], "missing comment file must block before GitHub mutation")
    finally:
        module.github_request = original_request
        if original_token is None:
            os.environ.pop("GRIMOIRE_GITHUB_PAT", None)
        else:
            os.environ["GRIMOIRE_GITHUB_PAT"] = original_token


def run_decide_fixture_contracts(actions_root: Path) -> None:
    script = helper(actions_root, "cast", "cast_driver.py")
    fixture_module = load_decide_fixture_module(actions_root)
    cases = fixture_module.load_decide_fixture_cases()
    expected_by_name = fixture_module.expected_decide_tuples()
    require(len(cases) == 7 and set(expected_by_name) == {str(case["name"]) for case in cases}, "decide fixture inventory must contain the seven Task 4 cases")
    root = make_workspace("decide-fixture-contracts")
    failure_cases = {"scope-violation-failure", "malformed-verdict-failure", "preflight-blocked-failure"}
    seen_failures: set[str] = set()
    for case in cases:
        name = str(case["name"])
        require(tuple(case["expected"]) == tuple(expected_by_name[name]), f"{name} must reuse expected_decide_tuples()")
        workspace = copy_decide_workspace(root, case)
        input_paths = case["input_paths"]
        run_helper(
            script,
            [
                "decide",
                "--consumer-workspace",
                str(workspace),
                "--preflight-status",
                input_paths["preflight"],
                "--review-status",
                input_paths["review"],
                "--review-outcome",
                str(case["review_outcome"]),
                "--design-status",
                input_paths["design"],
                "--issue-status",
                input_paths["issues"],
                "--spec-gap-status",
                input_paths["spec_gap"],
                "--fix-status",
                input_paths["fix"],
                "--fix-outcome",
                str(case["fix_outcome"]),
                "--boulder-status",
                input_paths["boulder"],
                "--verdict-status",
                input_paths["verdict"],
                "--verify-outcome",
                str(case["verify_outcome"]),
                "--output",
                ".omo/ci/cast-decision.json",
            ],
            {0},
        )
        decision = read_json(workspace, ".omo/ci/cast-decision.json")
        actual = fixture_module.actual_decide_tuple(decision)
        require(actual == tuple(case["expected"]), f"{name} tuple mismatch: actual={actual} expected={case['expected']}")
        expected_exit, expected_status, expected_conclusion = complete_status_for(decision)
        push_status = None
        if decision.get("decision") == "scoped-push":
            push_status = ".omo/ci/cast-push-status.json"
            write_json(workspace, push_status, {"schema_version": 1, "stage": "grimoire-cast", "status": "pushed", "push_count": 1})
        final = run_complete_case(script, workspace, ".omo/ci/cast-decision.json", ".omo/ci/cast-final.json", expected_exit, expected_status, expected_conclusion, push_status)
        require(final["decision"] == decision["decision"], f"{name} complete must preserve decision")
        if name in failure_cases:
            require(decision["exit_code"] == 1 and decision["conclusion"] == "failure", f"{name} must remain red failure")
            seen_failures.add(name)
        if name in {"spec-gap-advisory", "no-actionable-work"}:
            require(decision["exit_code"] == 0 and decision["conclusion"] == "neutral" and decision["label_transition"] == "spec-needed", f"{name} must be advisory spec-needed")
    require(seen_failures == failure_cases, "scope-violation, malformed verdict, and preflight-blocked fixtures must all remain red")


def assert_cast(actions_root: Path, workspace: Path, clear_fix: str, fixed_fix: str, approve_verdict: str) -> None:
    script = helper(actions_root, "cast", "cast_driver.py")
    run_decide_fixture_contracts(actions_root)
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

    protected = dict(trusted)
    protected.update({"status": "protected", "action": "halt", "model_execution_allowed": False, "write_allowed": False, "commit_allowed": False, "push_allowed": False, "github_mutation_allowed": False})
    write_json(workspace, ".omo/ci/trusted-controller-protected.json", protected)
    run_helper(
        script,
        [
            "preflight",
            "--consumer-workspace",
            str(workspace),
            "--trusted-status-path",
            ".omo/ci/trusted-controller-protected.json",
            "--trusted-outcome",
            "success",
            "--trusted-status",
            "protected",
            "--trusted-action",
            "halt",
            "--model-execution-allowed",
            "false",
            "--write-allowed",
            "false",
            "--commit-allowed",
            "false",
            "--push-allowed",
            "false",
            "--github-mutation-allowed",
            "false",
            "--output",
            ".omo/ci/cast-preflight-protected.json",
        ],
        {0},
    )
    protected_preflight = read_json(workspace, ".omo/ci/cast-preflight-protected.json")
    require(protected_preflight["status"] == "blocked" and protected_preflight["can_continue"] is False, "protected trusted-controller status must block cast preflight")

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
    require(final_clear["status"] == "terminal" and final_clear["conclusion"] == "success", "clear-noop complete status must be terminal success")
    require(isinstance(final_clear.get("summary"), str) and final_clear["summary"], "clear-noop complete must emit summary")

    run_helper(
        script,
        [
            "decide",
            "--consumer-workspace",
            str(workspace),
            "--preflight-status",
            ".omo/ci/cast-preflight-protected.json",
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
            ".omo/ci/cast-decision-protected.json",
        ],
        {0},
    )
    protected_decision = read_json(workspace, ".omo/ci/cast-decision-protected.json")
    require(protected_decision["exit_code"] == 1 and protected_decision["conclusion"] == "failure", "protected preflight failures must remain red")
    run_complete_case(script, workspace, ".omo/ci/cast-decision-protected.json", ".omo/ci/cast-final-protected.json", 1, "fizzled", "failure")

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
    require(final_fixed["conclusion"] == "success" and isinstance(final_fixed.get("summary"), str) and final_fixed["summary"], "fixed complete must emit success conclusion and summary")
    write_json(workspace, ".omo/ci/cast-push-status-bad-count.json", {"schema_version": 1, "stage": "grimoire-cast", "status": "pushed", "push_count": 2})
    run_complete_case(script, workspace, ".omo/ci/cast-decision-fixed.json", ".omo/ci/cast-final-bad-count.json", 1, "fizzled", "failure", ".omo/ci/cast-push-status-bad-count.json")
    write_json(workspace, ".omo/ci/cast-decision-invalid-conclusion.json", {"schema_version": 1, "stage": "grimoire-cast", "status": "ok", "decision": "clear-noop-terminal", "conclusion": "invalid", "terminal": True, "should_push": False, "exit_code": 0, "reasons": []})
    run_complete_case(script, workspace, ".omo/ci/cast-decision-invalid-conclusion.json", ".omo/ci/cast-final-invalid-conclusion.json", 1, "fizzled", "failure")

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


def assert_runtime_artifact_exclusions(actions_root: Path) -> None:
    fix_module = load_script_module(helper(actions_root, "fix", "fix.py"))
    verify_module = load_script_module(helper(actions_root, "verify", "verify.py"))
    cast_module = load_script_module(helper(actions_root, "cast", "cast_driver.py"))
    for module_name, module in (("fix", fix_module), ("verify", verify_module), ("cast", cast_module)):
        workspace = make_workspace(f"{module_name}-omo-filter")
        subprocess.run(["git", "init"], cwd=str(workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        rel_path(workspace, "docs/real-change.md").parent.mkdir(parents=True, exist_ok=True)
        rel_path(workspace, "docs/real-change.md").write_text("baseline\n", encoding="utf-8")
        subprocess.run(["git", "config", "user.name", "grimoire-test"], cwd=str(workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        subprocess.run(["git", "config", "user.email", "grimoire-test@dongwontuna-labs.invalid"], cwd=str(workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        subprocess.run(["git", "add", "docs/real-change.md"], cwd=str(workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        subprocess.run(["git", "commit", "-m", "test: baseline"], cwd=str(workspace), check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        rel_path(workspace, ".omo/ci/runtime.json").parent.mkdir(parents=True, exist_ok=True)
        rel_path(workspace, ".omo/ci/runtime.json").write_text("{}\n", encoding="utf-8")
        rel_path(workspace, "docs/real-change.md").write_text("real change\n", encoding="utf-8")
        paths = module.git_changed_paths(workspace) if module_name in {"fix", "verify"} else module.changed_paths(workspace)
        require(".omo" not in paths and all(not str(path).startswith(".omo/") for path in paths), f"{module_name} scanner must exclude .omo runtime artifacts")
        require("docs/real-change.md" in paths, f"{module_name} scanner must keep real changed files")


def assert_stage_contract(actions_root: Path) -> None:
    workspace = make_workspace("stage-contract")
    assert_setup_opencode(actions_root, workspace)
    assert_review(actions_root, workspace)
    assert_live_review_diagnostics(actions_root, workspace)
    assert_design_and_issues(actions_root, workspace)
    assert_spec_gap(actions_root, workspace)
    assert_runtime_artifact_exclusions(actions_root)
    assert_spec_gap_comment_upsert(actions_root, workspace)
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
