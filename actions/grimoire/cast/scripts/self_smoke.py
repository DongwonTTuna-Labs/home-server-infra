#!/usr/bin/env python3
# pyright: reportAny=false, reportExplicitAny=false, reportUnusedCallResult=false, reportImplicitStringConcatenation=false, reportUnknownVariableType=false
from __future__ import annotations

import argparse
import importlib.util
import json
import os
import pathlib
import sys
import tempfile
from datetime import datetime, timezone
from types import ModuleType
from typing import Any

STAGE = "grimoire-self-smoke"
DEFAULT_CASES = ("spec-gap-advisory", "clear-noop-success", "scope-violation-failure")
FINAL_EXPECTATIONS: dict[str, dict[str, object]] = {
    "spec-gap-advisory": {
        "final_status": "advisory",
        "final_conclusion": "neutral",
        "complete_exit_code": 0,
        "terminal": False,
        "should_push": False,
        "label_transition": "spec-needed",
    },
    "clear-noop-success": {
        "final_status": "terminal",
        "final_conclusion": "success",
        "complete_exit_code": 0,
        "terminal": True,
        "should_push": False,
        "label_transition": "done",
    },
    "scope-violation-failure": {
        "final_status": "fizzled",
        "final_conclusion": "failure",
        "complete_exit_code": 1,
        "terminal": False,
        "should_push": False,
        "label_transition": "fizzled",
    },
}


class SmokeError(Exception):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def artifact_parent_candidates() -> list[pathlib.Path]:
    raw_candidates = [
        os.environ.get("RUNNER_TEMP"),
        os.environ.get("TMPDIR"),
        os.environ.get("TEMP"),
        os.environ.get("TMP"),
        tempfile.gettempdir(),
        "/tmp",
        "/var/tmp",
        str(pathlib.Path.cwd() / ".grimoire-self-smoke-tmp"),
    ]
    candidates: list[pathlib.Path] = []
    seen: set[str] = set()
    for raw in raw_candidates:
        if not raw:
            continue
        path = pathlib.Path(raw).expanduser()
        key = str(path)
        if key in seen:
            continue
        candidates.append(path)
        seen.add(key)
    return candidates


def default_artifact_root() -> pathlib.Path:
    failures: list[str] = []
    for root in artifact_parent_candidates():
        try:
            root.mkdir(parents=True, exist_ok=True)
            return pathlib.Path(tempfile.mkdtemp(prefix="grimoire-self-smoke-", dir=str(root)))
        except OSError as exc:
            failures.append(f"{root}: {exc}")
    raise SmokeError("unable to create artifact root under any candidate temp parent: " + "; ".join(failures))


def prepare_artifact_root(raw: str) -> pathlib.Path:
    if not raw:
        return default_artifact_root()
    artifact_root = pathlib.Path(raw).resolve()
    if artifact_root.exists() and any(artifact_root.iterdir()):
        raise SmokeError(f"artifact root must be empty before self-smoke: {artifact_root}")
    artifact_root.mkdir(parents=True, exist_ok=True)
    return artifact_root


def fixture_helper_path(root: pathlib.Path) -> pathlib.Path:
    path = root / "tests" / "fixtures" / "grimoire" / "run-loop-fixtures.py"
    if not path.is_file():
        raise SmokeError(f"missing fixture helper: {path}")
    return path


def load_fixture_helper(root: pathlib.Path) -> ModuleType:
    path = fixture_helper_path(root)
    spec = importlib.util.spec_from_file_location("grimoire_run_loop_fixtures", path)
    if spec is None or spec.loader is None:
        raise SmokeError(f"unable to load fixture helper: {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module



def require(condition: bool, message: str) -> None:
    if not condition:
        raise SmokeError(message)


def expect_equal(actual: object, expected: object, message: str) -> None:
    if actual != expected:
        raise SmokeError(f"{message}: actual={actual!r} expected={expected!r}")


def expected_int(expectations: dict[str, object], key: str) -> int:
    value = expectations[key]
    if isinstance(value, bool) or not isinstance(value, int):
        raise SmokeError(f"expectation {key} must be an integer")
    return value


def decide_args(helper: ModuleType, workspace: pathlib.Path, case: dict[str, Any]) -> list[str]:
    paths = getattr(helper, "DECIDE_INPUT_PATHS")
    return [
        "decide",
        "--consumer-workspace",
        str(workspace),
        "--preflight-status",
        paths["preflight"],
        "--review-status",
        paths["review"],
        "--review-outcome",
        str(case["review_outcome"]),
        "--design-status",
        paths["design"],
        "--issue-status",
        paths["issues"],
        "--spec-gap-status",
        paths["spec_gap"],
        "--fix-status",
        paths["fix"],
        "--fix-outcome",
        str(case["fix_outcome"]),
        "--boulder-status",
        paths["boulder"],
        "--verdict-status",
        paths["verdict"],
        "--verify-outcome",
        str(case["verify_outcome"]),
        "--output",
        getattr(helper, "DECIDE_OUTPUT"),
    ]


def read_workspace_json(helper: ModuleType, workspace: pathlib.Path, relative: str) -> dict[str, Any]:
    read_json = getattr(helper, "read_json")
    payload = read_json(workspace, relative)
    if not isinstance(payload, dict):
        raise SmokeError(f"fixture helper returned non-object JSON: {relative}")
    return payload


def run_case(helper: ModuleType, artifact_root: pathlib.Path, case: dict[str, Any], log_lines: list[str]) -> dict[str, Any]:
    name = str(case["name"])
    expected_final = FINAL_EXPECTATIONS[name]
    getattr(helper, "assert_decide_fixture_inventory")(case)
    workspace = getattr(helper, "copy_decide_fixture_workspace")(artifact_root, case)
    getattr(helper, "run_helper")("cast", "cast_driver.py", decide_args(helper, workspace, case), {0}, workspace, log_lines)

    decision_path = getattr(helper, "DECIDE_OUTPUT")
    decision = read_workspace_json(helper, workspace, decision_path)
    actual_tuple = getattr(helper, "actual_decide_tuple")(decision)
    expected_tuple = tuple(case["expected"])
    expect_equal(actual_tuple, expected_tuple, f"{name} decision tuple mismatch")
    expect_equal(decision.get("label_transition"), expected_final["label_transition"], f"{name} label transition mismatch")
    expect_equal(decision.get("terminal"), expected_final["terminal"], f"{name} terminal mismatch")
    expect_equal(decision.get("should_push"), expected_final["should_push"], f"{name} should_push mismatch")

    if name == "spec-gap-advisory":
        paths = getattr(helper, "DECIDE_INPUT_PATHS")
        spec_gap = read_workspace_json(helper, workspace, paths["spec_gap"])
        expect_equal(spec_gap.get("no_code_or_push_action"), True, "spec-gap advisory must forbid code or push action")
        expect_equal(decision.get("decision"), "spec-gap-halt", "spec-gap advisory decision mismatch")
        expect_equal(decision.get("conclusion"), "neutral", "spec-gap advisory conclusion mismatch")
        expect_equal(decision.get("label_transition"), "spec-needed", "spec-gap advisory label mismatch")

    final_path = f".omo/ci/cast-final-{name}.json"
    complete_exit_code = expected_int(expected_final, "complete_exit_code")
    completed = getattr(helper, "run_helper")(
        "cast",
        "cast_driver.py",
        ["complete", "--consumer-workspace", str(workspace), "--decision", decision_path, "--output", final_path],
        {complete_exit_code},
        workspace,
        log_lines,
    )
    final = read_workspace_json(helper, workspace, final_path)
    expect_equal(completed.returncode, complete_exit_code, f"{name} complete process exit mismatch")
    expect_equal(final.get("status"), expected_final["final_status"], f"{name} final status mismatch")
    expect_equal(final.get("conclusion"), expected_final["final_conclusion"], f"{name} final conclusion mismatch")
    expect_equal(final.get("decision"), decision.get("decision"), f"{name} final decision mismatch")
    expect_equal(final.get("should_push"), expected_final["should_push"], f"{name} final should_push mismatch")
    require(not (workspace / ".omo" / "ci" / "cast-push-status.json").exists(), f"{name} must not write or require push status")

    return {
        "name": name,
        "decision": decision.get("decision"),
        "conclusion": decision.get("conclusion"),
        "decision_exit_code": decision.get("exit_code"),
        "label_transition": decision.get("label_transition"),
        "terminal": decision.get("terminal"),
        "should_push": decision.get("should_push"),
        "final_status": final.get("status"),
        "final_conclusion": final.get("conclusion"),
        "complete_exit_code": completed.returncode,
        "code_action_attempted": False,
        "external_push_attempted": False,
        "workspace": workspace.relative_to(artifact_root).as_posix(),
    }


def run_signal_smoke(root: pathlib.Path, artifact_root: pathlib.Path) -> dict[str, Any]:
    helper = load_fixture_helper(root)
    helper_temp_root = artifact_root / "fixture-temp"
    setattr(helper, "TEMP_ROOT", helper_temp_root)
    helper_temp_root.mkdir(parents=True, exist_ok=True)
    cases_by_name = {str(case["name"]): case for case in getattr(helper, "load_decide_fixture_cases")()}
    missing = [name for name in DEFAULT_CASES if name not in cases_by_name]
    if missing:
        raise SmokeError("missing required decide fixture cases: " + ", ".join(missing))
    log_lines = [
        "# Grimoire self-smoke deterministic signal layer",
        f"generated_at={utc_now()}",
        f"control_plane_root={root}",
        "live_opencode_invoked=false",
        "external_push_attempted=false",
        "secret_values=redacted",
        "",
    ]
    summaries = [run_case(helper, artifact_root, cases_by_name[name], log_lines) for name in DEFAULT_CASES]
    summary = {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "status": "ok",
        "artifact_root": str(artifact_root),
        "log_path": str(artifact_root / "self-smoke.log"),
        "case_count": len(summaries),
        "cases": summaries,
        "deterministic_signal_layer": True,
        "live_opencode_invoked": False,
        "model_output_dependency": False,
        "real_consumer_credentials_required": False,
        "external_push_attempted": False,
        "real_remote_mutation_attempted": False,
    }
    write_json(artifact_root / "self-smoke-summary.json", summary)
    (artifact_root / "self-smoke.log").write_text("\n".join(log_lines).rstrip() + "\n", encoding="utf-8")
    return summary


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run deterministic Grimoire signal-layer self-smoke cases without live opencode or external push.")
    parser.add_argument("--control-plane-root", default=".")
    parser.add_argument("--artifact-root", default="")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    root = pathlib.Path(args.control_plane_root).resolve()
    try:
        artifact_root = prepare_artifact_root(str(args.artifact_root))
        summary = run_signal_smoke(root, artifact_root)
    except SmokeError as exc:
        print(f"{STAGE}: failed: {exc}", file=sys.stderr)
        return 1
    except Exception as exc:
        print(f"{STAGE}: failed: {exc}", file=sys.stderr)
        return 1

    print(f"{STAGE}: status=ok artifact_root={summary['artifact_root']}")
    for case in summary["cases"]:
        print(
            (
                f"PASS {case['name']}: decision={case['decision']} "
                f"conclusion={case['conclusion']} final_status={case['final_status']} "
                f"complete_exit={case['complete_exit_code']} external_push_attempted=false"
            )
        )
    print(f"summary={artifact_root / 'self-smoke-summary.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
