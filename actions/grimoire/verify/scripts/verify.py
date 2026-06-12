#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import sys
from datetime import datetime, timezone

STAGE = "grimoire-verify"
LENSES = ("f1_oracle", "f2_quality", "f3_real_qa", "f4_scope")
ENUM_VALUES = {"APPROVE", "REJECT"}
JQ_APPROVE_EXPR = 'type == "object" and .schema_version == 1 and .stage == "grimoire-verify" and (.notes | type == "object") and (.notes.f1_oracle | type == "object") and (.notes.f2_quality | type == "object") and (.notes.f3_real_qa | type == "object") and (.notes.f4_scope | type == "object") and .f1_oracle == "APPROVE" and .f2_quality == "APPROVE" and .f3_real_qa == "APPROVE" and .f4_scope == "APPROVE" and .approved == true'


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
        statuses = {lens: "REJECT" for lens in LENSES}
        exit_code = 1
    payload = payload_for(args.fixture or "blocked", statuses, workspace, output, spec, gap, fix)
    if not args.fixture:
        payload["blocked_reason"] = "live verification is wired by later workflow tasks; default action path fails closed"
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
