#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
from datetime import datetime, timezone

STAGE = "grimoire-verify"
LENSES = ("f1_oracle", "f2_quality", "f3_real_qa", "f4_scope")
ENUM_VALUES = {"APPROVE", "REJECT"}
JQ_APPROVE_EXPR = 'type == "object" and .schema_version == 1 and .stage == "grimoire-verify" and (.notes | type == "object") and (.notes.f1_oracle | type == "object") and (.notes.f2_quality | type == "object") and (.notes.f3_real_qa | type == "object") and (.notes.f4_scope | type == "object") and .f1_oracle == "APPROVE" and .f2_quality == "APPROVE" and .f3_real_qa == "APPROVE" and .f4_scope == "APPROVE" and .approved == true'


class ContractError(Exception):
    pass


MARKER_PATH = "docs/GRIMOIRE_PUSH_SMOKE.md"
MARKER_SPEC_PATH = "docs/GRIMOIRE_PUSH_SMOKE.spec.md"
FORBIDDEN_PATH_PREFIXES = (".github/", ".opencode/", "src/")
FORBIDDEN_PATHS = {"Cargo.toml", "Cargo.lock", "opencode.json"}


def load_json(path: pathlib.Path, label: str) -> dict[str, object]:
    if not path.exists():
        raise ContractError(f"{label} does not exist: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"{label} is not valid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise ContractError(f"{label} must be a JSON object")
    return payload


def run_git(args: list[str], workspace: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=str(workspace), check=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def git_changed_paths(workspace: pathlib.Path) -> list[str]:
    result = run_git(["status", "--porcelain", "--untracked-files=all"], workspace)
    if result.returncode != 0:
        raise ContractError("git status failed while verifying fix result")
    paths: list[str] = []
    seen: set[str] = set()
    for line in result.stdout.splitlines():
        if not line:
            continue
        raw = line[3:].strip()
        if " -> " in raw:
            raw = raw.split(" -> ", 1)[1]
        if raw and not raw.startswith(".omo/") and raw not in seen:
            paths.append(raw)
            seen.add(raw)
    return paths


def canonical_marker_from_spec(path: pathlib.Path) -> str | None:
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8", errors="replace")
    match = re.search(r"```markdown\n(.*?)\n```", text, flags=re.DOTALL)
    if match is None:
        return None
    return match.group(1).rstrip() + "\n"


def status(value: bool) -> str:
    return "APPROVE" if value else "REJECT"


def evidence_note(verdict: str, summary: str, evidence: list[str]) -> dict[str, object]:
    return note(verdict, summary, evidence)


def real_payload(workspace: pathlib.Path, output: pathlib.Path, spec_path: pathlib.Path, gap_path: pathlib.Path, fix_path: pathlib.Path) -> tuple[dict[str, object], int]:
    spec = load_json(spec_path, "spec sufficiency artifact")
    gap = load_json(gap_path, "spec-gap status artifact") if gap_path.exists() else {"should_halt": False}
    fix = load_json(fix_path, "fix status artifact")
    changed = git_changed_paths(workspace)
    fix_changed_raw = fix.get("changed_files")
    allowed_raw = fix.get("allowed_paths")
    allowed_writes_raw = fix.get("allowed_write_paths")
    if not isinstance(allowed_writes_raw, list):
        allowed_writes_raw = fix.get("spec_target_paths")
    fix_changed = [str(item) for item in fix_changed_raw] if isinstance(fix_changed_raw, list) else []
    allowed = [str(item) for item in allowed_raw] if isinstance(allowed_raw, list) else []
    allowed_writes = [str(item) for item in allowed_writes_raw] if isinstance(allowed_writes_raw, list) else []

    prereq_ok = spec.get("spec_sufficient") is True and gap.get("should_halt") is not True and fix.get("scope_ok") is True and fix.get("status") in {"clear-noop", "fixed"}
    changed_match = sorted(changed) == sorted(fix_changed)
    allowed_exact = all(path in allowed for path in changed)
    forbidden_paths = [path for path in changed if path in FORBIDDEN_PATHS or path.startswith(FORBIDDEN_PATH_PREFIXES)]
    write_authorized = fix.get("status") == "clear-noop" or bool(changed) and all(path in allowed_writes for path in changed)

    marker_ok = True
    marker_evidence = "no marker change required"
    if MARKER_PATH in changed:
        canonical = canonical_marker_from_spec(workspace / MARKER_SPEC_PATH)
        actual_path = workspace / MARKER_PATH
        if canonical is None:
            marker_ok = False
            marker_evidence = f"{MARKER_SPEC_PATH} missing canonical markdown block"
        elif not actual_path.exists():
            marker_ok = False
            marker_evidence = f"{MARKER_PATH} missing after fix"
        else:
            actual = actual_path.read_text(encoding="utf-8", errors="replace")
            marker_ok = actual == canonical
            marker_evidence = f"{MARKER_PATH} {'matches' if marker_ok else 'does not match'} canonical markdown"

    f1 = prereq_ok and changed_match
    f2 = marker_ok and (fix.get("status") == "clear-noop" or bool(changed))
    f3 = changed_match and all((workspace / path).exists() for path in changed)
    f4 = allowed_exact and write_authorized and not forbidden_paths and not fix.get("violations")
    statuses = {
        "f1_oracle": status(f1),
        "f2_quality": status(f2),
        "f3_real_qa": status(f3),
        "f4_scope": status(f4),
    }
    notes = {
        "f1_oracle": evidence_note(statuses["f1_oracle"], "Spec, gap, fix, and git status artifacts are mutually consistent.", [f"spec_sufficient={spec.get('spec_sufficient')}", f"fix_status={fix.get('status')}", f"changed_match={changed_match}"]),
        "f2_quality": evidence_note(statuses["f2_quality"], "Changed marker content matches the canonical directive and non-noop fixes are non-empty.", [marker_evidence, f"changed_files={changed}"]),
        "f3_real_qa": evidence_note(statuses["f3_real_qa"], "Verification inspected the actual post-fix working tree paths.", [f"git_status_paths={changed}"]),
        "f4_scope": evidence_note(statuses["f4_scope"], "Every changed path is explicitly allowed and write-authorized by design/fix scope metadata.", [f"allowed_write_paths={allowed_writes}", f"forbidden_paths={forbidden_paths}"]),
    }
    payload: dict[str, object] = {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "fixture": "none",
        "approved": approved(statuses),
        "f1_oracle": statuses["f1_oracle"],
        "f2_quality": statuses["f2_quality"],
        "f3_real_qa": statuses["f3_real_qa"],
        "f4_scope": statuses["f4_scope"],
        "notes": notes,
        "prerequisites": {"spec_sufficiency": rel(spec_path, workspace), "spec_gap_status": rel(gap_path, workspace), "fix_status": rel(fix_path, workspace)},
        "changed_files": changed,
        "allowed_write_paths": allowed_writes,
        "jq_all_approve_predicate": JQ_APPROVE_EXPR,
        "output_path": rel(output, workspace),
        "real_verification_attempted": True,
    }
    return payload, 0 if payload["approved"] is True else 1


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def resolve_path(raw: str, workspace: pathlib.Path) -> pathlib.Path:
    path = pathlib.Path(raw)
    if path.is_absolute():
        return path
    return workspace / path


def rel(path: pathlib.Path, root: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except (OSError, ValueError):
        return path.as_posix()


def note(status: str, summary: str, evidence: list[str]) -> dict[str, object]:
    return {"status": status, "summary": summary, "evidence": evidence, "reviewer": STAGE}


def approved(statuses: dict[str, str]) -> bool:
    return all(statuses.get(lens) == "APPROVE" for lens in LENSES)


def payload_for(fixture: str, statuses: dict[str, str], workspace: pathlib.Path, output: pathlib.Path, spec: pathlib.Path, gap: pathlib.Path, fix: pathlib.Path) -> dict[str, object]:
    notes = {
        lens: note(statuses[lens], f"{lens} returned {statuses[lens]} for {fixture} fixture.", [f"deterministic {fixture} fixture"])
        for lens in LENSES
    }
    return {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "fixture": fixture,
        "approved": approved(statuses),
        "f1_oracle": statuses["f1_oracle"],
        "f2_quality": statuses["f2_quality"],
        "f3_real_qa": statuses["f3_real_qa"],
        "f4_scope": statuses["f4_scope"],
        "notes": notes,
        "prerequisites": {
            "spec_sufficiency": rel(spec, workspace),
            "spec_gap_status": rel(gap, workspace),
            "fix_status": rel(fix, workspace),
        },
        "jq_all_approve_predicate": JQ_APPROVE_EXPR,
        "output_path": rel(output, workspace),
    }


def write_json(path: pathlib.Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_payload(payload: object) -> bool:
    if not isinstance(payload, dict):
        return False
    if payload.get("schema_version") != 1 or payload.get("stage") != STAGE:
        return False
    notes = payload.get("notes")
    if not isinstance(notes, dict):
        return False
    for lens in LENSES:
        if payload.get(lens) not in ENUM_VALUES:
            return False
        if not isinstance(notes.get(lens), dict):
            return False
    return approved({lens: str(payload.get(lens)) for lens in LENSES}) and payload.get("approved") is True


def validate_file(path: pathlib.Path) -> int:
    if not path.exists():
        print(f"{STAGE}: verdict missing: {path}", file=sys.stderr)
        return 1
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        print(f"{STAGE}: verdict malformed: {exc}", file=sys.stderr)
        return 1
    if validate_payload(payload):
        print(f"{STAGE}: all-APPROVE predicate accepted {path}")
        return 0
    print(f"{STAGE}: all-APPROVE predicate rejected {path}", file=sys.stderr)
    return 1


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            text = "true" if value is True else "false" if value is False else str(value)
            handle.write(f"{key}={text}\n")


def run(args: argparse.Namespace) -> int:
    if args.jq_expression:
        print(JQ_APPROVE_EXPR)
        return 0
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    output = resolve_path(args.output, workspace)
    spec = resolve_path(args.spec_sufficiency, workspace)
    gap = resolve_path(args.spec_gap_status, workspace)
    fix = resolve_path(args.fix_status, workspace)
    if args.validate:
        return validate_file(resolve_path(args.validate, workspace))
    if args.fixture == "approve":
        statuses = {lens: "APPROVE" for lens in LENSES}
        exit_code = 0
    elif args.fixture == "reject":
        statuses = {"f1_oracle": "APPROVE", "f2_quality": "REJECT", "f3_real_qa": "REJECT", "f4_scope": "APPROVE"}
        exit_code = 1
    elif args.fixture == "invalid":
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps({"schema_version": 1, "stage": STAGE, "approved": True}, indent=2) + "\n", encoding="utf-8")
        write_github_output(args.github_output, {"approved": False, "output_path": str(output), "jq_predicate": JQ_APPROVE_EXPR})
        print(f"{STAGE}: wrote invalid fixture to {output}", file=sys.stderr)
        return 1
    else:
        try:
            payload, exit_code = real_payload(workspace, output, spec, gap, fix)
        except ContractError as exc:
            statuses = {lens: "REJECT" for lens in LENSES}
            payload = payload_for("blocked", statuses, workspace, output, spec, gap, fix)
            payload["blocked_reason"] = str(exc)
            payload["real_verification_attempted"] = True
            exit_code = 1
            write_json(output, payload)
            write_github_output(args.github_output, {"approved": payload["approved"], "output_path": str(output), "jq_predicate": JQ_APPROVE_EXPR})
            print(f"{STAGE}: approved={str(payload['approved']).lower()} output={output}")
            return exit_code
        write_json(output, payload)
        write_github_output(args.github_output, {"approved": payload["approved"], "output_path": str(output), "jq_predicate": JQ_APPROVE_EXPR})
        print(f"{STAGE}: approved={str(payload['approved']).lower()} output={output}")
        return exit_code
    payload = payload_for(args.fixture or "blocked", statuses, workspace, output, spec, gap, fix)
    write_json(output, payload)
    write_github_output(args.github_output, {"approved": payload["approved"], "output_path": str(output), "jq_predicate": JQ_APPROVE_EXPR})
    print(f"{STAGE}: approved={str(payload['approved']).lower()} output={output}")
    return exit_code


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run or validate the Grimoire F1-F4 verification stage.")
    parser.add_argument("--jq-expression", action="store_true")
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--output", default=".omo/grimoire/verdict.json")
    parser.add_argument("--fixture", choices=["approve", "reject", "invalid"], default="")
    parser.add_argument("--validate", default="")
    parser.add_argument("--spec-sufficiency", default=".omo/ci/spec-sufficiency.json")
    parser.add_argument("--spec-gap-status", default=".omo/ci/spec-gap-status.json")
    parser.add_argument("--fix-status", default=".omo/ci/fix-status.json")
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
