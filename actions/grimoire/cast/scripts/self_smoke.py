#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone

STAGE = "grimoire-self-smoke"
TEMP_ROOT = pathlib.Path("/var/folders/vz/hx33c759727ftq88cxbgp8r40000gn/T/opencode")

PROPOSAL = """# Change: Grimoire Push Smoke

## Why
We need one live positive smoke proving the merged Grimoire reusable control plane can perform a benign docs-only scoped autofix from a normal same-repo pull request, push exactly one bot commit, and re-review on `pull_request.synchronize`.

## What Changes
- Add an OpenSpec-backed smoke fixture for `grimoire-push-smoke`.
- Add a deterministic directive at `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.
- Intentionally leave `docs/GRIMOIRE_PUSH_SMOKE.md` absent so Grimoire can create it.
"""

DESIGN = """# Design: Grimoire Push Smoke

## Scope
This change is a smoke fixture only. The active scope is limited to proving the Grimoire reusable control-plane live positive path on a benign documentation addition.

## Expected Automation Behavior
1. Review/design should notice that `docs/GRIMOIRE_PUSH_SMOKE.spec.md` requires `docs/GRIMOIRE_PUSH_SMOKE.md`.
2. Design should classify the missing marker as in scope for this smoke.
3. Fix should create only `docs/GRIMOIRE_PUSH_SMOKE.md` with the canonical content in `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.
4. Verify should approve only if the patch is additive, docs-only, and limited to the marker file.
5. Cast should reach a scoped-push decision, but this self-smoke must not push to any external consumer repo.
"""

TASKS = """# Tasks

- [ ] 1. Create `docs/GRIMOIRE_PUSH_SMOKE.md` with the exact canonical Markdown from `docs/GRIMOIRE_PUSH_SMOKE.spec.md`. This task is complete when the marker file exists with exact content; Grimoire must not edit this task checkbox.
"""

SPEC = """## ADDED Requirements

### Requirement: Grimoire push smoke marker
The repository SHALL contain `docs/GRIMOIRE_PUSH_SMOKE.md` with the exact canonical Markdown content specified in `docs/GRIMOIRE_PUSH_SMOKE.spec.md`.

#### Scenario: Grimoire recreates the missing marker
- **GIVEN** a same-repo non-draft pull request for the `grimoire-push-smoke` change
- **AND** `docs/GRIMOIRE_PUSH_SMOKE.spec.md` exists
- **AND** `docs/GRIMOIRE_PUSH_SMOKE.md` is absent before Grimoire runs
- **WHEN** Grimoire runs review, design, fix, verify, and cast
- **THEN** Grimoire creates exactly `docs/GRIMOIRE_PUSH_SMOKE.md` with the canonical content from `docs/GRIMOIRE_PUSH_SMOKE.spec.md`
- **AND** Grimoire does not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, or live-capable venue behavior
"""

DOC_SPEC = """# Grimoire Push Smoke Spec

## Purpose
This tracked spec is the deterministic smoke directive for the Grimoire reusable control-plane live push smoke.

## Target File
`docs/GRIMOIRE_PUSH_SMOKE.md`

## Required Grimoire Action
If `docs/GRIMOIRE_PUSH_SMOKE.md` is absent, create it with exactly the canonical Markdown below and do not change any other file.

## Canonical Markdown

```markdown
# Grimoire Push Smoke

This file documents the Grimoire reusable control-plane push smoke for the OpenSpec-backed `grimoire-push-smoke` change.

The smoke checks that Grimoire can compare the OpenSpec task and requirement context with the PR tree, notice that `docs/GRIMOIRE_PUSH_SMOKE.md` is missing, and route that finding through design, fix, verify, and scoped push.

The expected fix is deliberately small: create this Markdown file as a docs-only additive change, without modifying or deleting existing files. The push stage then commits and pushes that new-file patch to prove the reusable Grimoire control plane can carry a benign documentation fix through to the PR branch and trigger a `pull_request.synchronize` re-review.

This smoke must not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, or live-capable venue behavior.
```

## Forbidden Changes
Do not modify source code, workflows, configuration, credentials, relayer behavior, signing, nonce handling, authentication, live-capable venue behavior, OpenSpec task checkboxes, or any file other than `docs/GRIMOIRE_PUSH_SMOKE.md`.
"""

class SmokeError(Exception):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write_json(path: pathlib.Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def run(command: list[str], cwd: pathlib.Path, expected: set[int] | None = None, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    expected_codes = {0} if expected is None else expected
    completed = subprocess.run(command, cwd=str(cwd), env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)
    if completed.returncode not in expected_codes:
        raise SmokeError(f"command failed ({completed.returncode}, expected {sorted(expected_codes)}): {' '.join(command)}\nstdout={completed.stdout}\nstderr={completed.stderr}")
    return completed


def load_json(path: pathlib.Path) -> dict[str, object]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise SmokeError(f"JSON artifact must be an object: {path}")
    return payload


def prepare_workspace(root: pathlib.Path) -> pathlib.Path:
    workspace = root / "consumer"
    workspace.mkdir(parents=True, exist_ok=True)
    run(["git", "init"], workspace)
    run(["git", "config", "user.name", "grimoire-self-smoke"], workspace)
    run(["git", "config", "user.email", "grimoire-self-smoke@dongwontuna-labs.invalid"], workspace)
    write(workspace / "openspec/changes/grimoire-push-smoke/proposal.md", PROPOSAL)
    write(workspace / "openspec/changes/grimoire-push-smoke/design.md", DESIGN)
    write(workspace / "openspec/changes/grimoire-push-smoke/tasks.md", TASKS)
    write(workspace / "openspec/changes/grimoire-push-smoke/specs/grimoire-push-smoke/spec.md", SPEC)
    write(workspace / "docs/GRIMOIRE_PUSH_SMOKE.spec.md", DOC_SPEC)
    run(["git", "add", "openspec", "docs/GRIMOIRE_PUSH_SMOKE.spec.md"], workspace)
    run(["git", "commit", "-m", "test(openspec): add grimoire push smoke fixture"], workspace)
    return workspace


def helper(root: pathlib.Path, stage: str, name: str) -> pathlib.Path:
    path = root / "actions" / "grimoire" / stage / "scripts" / name
    if not path.is_file():
        raise SmokeError(f"missing helper: {path}")
    return path


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Run the real-opencode Grimoire self-smoke without external push.")
    parser.add_argument("--control-plane-root", default=".")
    parser.add_argument("--artifact-root", default="")
    args = parser.parse_args(argv)
    root = pathlib.Path(args.control_plane_root).resolve()
    artifact_root = pathlib.Path(args.artifact_root).resolve() if args.artifact_root else pathlib.Path(tempfile.mkdtemp(prefix="grimoire-self-smoke-", dir=str(TEMP_ROOT)))
    if artifact_root.exists():
        shutil.rmtree(artifact_root)
    artifact_root.mkdir(parents=True, exist_ok=True)
    workspace = prepare_workspace(artifact_root)
    env = os.environ.copy()
    env.setdefault("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
    env.setdefault("OPENCODE_PURE", "1")

    changed = workspace / ".omo/ci/changed-files.txt"
    write(changed, "openspec/changes/grimoire-push-smoke/proposal.md\nopenspec/changes/grimoire-push-smoke/design.md\nopenspec/changes/grimoire-push-smoke/tasks.md\nopenspec/changes/grimoire-push-smoke/specs/grimoire-push-smoke/spec.md\ndocs/GRIMOIRE_PUSH_SMOKE.spec.md\n")
    trusted = workspace / ".omo/ci/trusted-controller-status.json"
    run(["python3", str(helper(root, "trusted-controller", "trusted_controller.py")), "--consumer-workspace", str(workspace), "--control-plane-root", str(root), "--changed-files", str(changed), "--output", str(trusted)], root, env=env)
    review = workspace / ".omo/ci/review-findings.json"
    run(["python3", str(helper(root, "review", "review.py")), "--consumer-workspace", str(workspace), "--control-plane-root", str(root), "--output", str(review)], root, env=env)
    if load_json(review).get("status") != "findings":
        raise SmokeError("self-smoke review must produce an in-scope missing-marker finding")
    design = workspace / ".omo/ci/spec-sufficiency.json"
    run(["python3", str(helper(root, "design", "design.py")), "--consumer-workspace", str(workspace), "--repository", "DongwonTTuna-Labs/rs-builder-relayer-client", "--review-input", str(review), "--output", str(design), "--plan", ".omo/ci/design-plan.md"], root, env=env)
    issues = workspace / ".omo/ci/out-of-scope-issues-status.json"
    run(["python3", str(helper(root, "cast", "cast_driver.py")), "file-issues", "--consumer-workspace", str(workspace), "--design-path", str(design), "--repository", "local-consumer", "--pr-number", "0", "--output", str(issues), "--ledger", ".omo/ci/out-of-scope-issues-ledger.json"], root, env=env)
    gap = workspace / ".omo/ci/spec-gap-status.json"
    write_json(gap, {"schema_version": 1, "stage": "grimoire-spec-gap", "status": "clear", "should_halt": False})
    fix = workspace / ".omo/ci/fix-status.json"
    run(["python3", str(helper(root, "fix", "fix.py")), "--consumer-workspace", str(workspace), "--control-plane-root", str(root), "--spec-sufficiency", str(design), "--spec-gap-status", str(gap), "--pr-touched", str(changed), "--output", str(fix), "--handoff-output", ".omo/ci/fix-handoff-prompt.md"], root, env=env)
    fix_payload = load_json(fix)
    if fix_payload.get("status") != "fixed" or fix_payload.get("changed_files") != ["docs/GRIMOIRE_PUSH_SMOKE.md"]:
        raise SmokeError("self-smoke fix must create exactly docs/GRIMOIRE_PUSH_SMOKE.md")
    boulder = workspace / ".omo/boulder.json"
    run(["python3", str(helper(root, "cast", "cast_driver.py")), "boulder", "--consumer-workspace", str(workspace), "--fix-status", str(fix), "--output", str(boulder)], root, env=env)
    verdict = workspace / ".omo/grimoire/verdict.json"
    run(["python3", str(helper(root, "verify", "verify.py")), "--consumer-workspace", str(workspace), "--spec-sufficiency", str(design), "--spec-gap-status", str(gap), "--fix-status", str(fix), "--output", str(verdict)], root, env=env)
    preflight = workspace / ".omo/ci/cast-preflight.json"
    run(["python3", str(helper(root, "cast", "cast_driver.py")), "preflight", "--consumer-workspace", str(workspace), "--trusted-status-path", str(trusted), "--trusted-outcome", "success", "--trusted-status", "ok", "--trusted-action", "continue", "--model-execution-allowed", "true", "--write-allowed", "true", "--commit-allowed", "true", "--push-allowed", "true", "--github-mutation-allowed", "true", "--output", str(preflight)], root, env=env)
    decision = workspace / ".omo/ci/cast-decision.json"
    run(["python3", str(helper(root, "cast", "cast_driver.py")), "decide", "--consumer-workspace", str(workspace), "--preflight-status", str(preflight), "--review-status", str(review), "--review-outcome", "success", "--design-status", str(design), "--issue-status", str(issues), "--spec-gap-status", str(gap), "--fix-status", str(fix), "--fix-outcome", "success", "--boulder-status", str(boulder), "--verdict-status", str(verdict), "--verify-outcome", "success", "--output", str(decision)], root, env=env)
    decision_payload = load_json(decision)
    if decision_payload.get("decision") != "scoped-push" or decision_payload.get("status") != "ok" or decision_payload.get("should_push") is not True:
        raise SmokeError("self-smoke did not reach scoped-push decision")
    summary: dict[str, object] = {"schema_version": 1, "stage": STAGE, "generated_at": utc_now(), "status": "ok", "decision": "scoped-push", "external_push_attempted": False, "artifact_root": str(artifact_root), "consumer_shape_head": "345708674ac358ae2974af2c645e67456eeb39cf"}
    write_json(artifact_root / "self-smoke-summary.json", summary)
    print(f"{STAGE}: decision=scoped-push status=ok artifact_root={artifact_root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
