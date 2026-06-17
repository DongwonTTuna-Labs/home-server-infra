#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import sys
from datetime import datetime, timezone

STAGE = "grimoire-spec-gap"
SECTIONS = [
    "Summary",
    "Intended Work",
    "Missing OpenSpec Evidence",
    "Suggested Spec Items",
    "How To Rerun",
]
REQUIRED_FIELDS = (
    "spec_sufficient",
    "bindings",
    "missing",
    "safety_default_gaps",
    "suggested_spec_patch",
    "plan_path",
    "halt_reason",
)


class ContractError(Exception):
    pass


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


def plain(value: object, fallback: str = "not provided") -> str:
    text = re.sub(r"\s+", " ", str(value or "").strip())
    return text or fallback


def quote_block(value: object, fallback: str) -> str:
    text = str(value or "").rstrip() or fallback
    return "\n".join("> " + line if line else ">" for line in text.splitlines())


def load_payload(path: pathlib.Path) -> dict[str, object]:
    if not path.exists():
        raise ContractError(f"input does not exist: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"input is not valid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise ContractError("design input must be a JSON object")
    for field in REQUIRED_FIELDS:
        if field not in payload:
            raise ContractError(f"design input missing root field: {field}")
    if not isinstance(payload.get("spec_sufficient"), bool):
        raise ContractError("spec_sufficient must be boolean")
    for field in ("bindings", "missing", "safety_default_gaps"):
        if not isinstance(payload.get(field), list):
            raise ContractError(f"{field} must be an array")
    return payload


def render_comment(payload: dict[str, object]) -> str:
    missing_raw = payload.get("missing")
    gaps_raw = payload.get("safety_default_gaps")
    missing: list[object] = missing_raw if isinstance(missing_raw, list) else []
    gaps: list[object] = gaps_raw if isinstance(gaps_raw, list) else []
    missing_lines = []
    for item in missing:
        if isinstance(item, dict):
            missing_lines.append(f"- {plain(item.get('location'))}: {plain(item.get('reason'))}")
    if not missing_lines:
        missing_lines.append("- No missing finding entries were provided, but spec_sufficient=false requires a halt.")
    gap_lines = []
    for item in gaps:
        if isinstance(item, dict):
            gap_lines.append(f"- {plain(item.get('scope'))}: {plain(item.get('required_default'))}")
    if not gap_lines:
        gap_lines.append("- Add explicit OpenSpec coverage for every in-scope finding before rerunning.")
    lines = [
        "<!-- grimoire-spec-gap -->",
        "## Summary",
        f"Grimoire paused automated code changes and recorded advisory, non-blocking guidance because {plain(payload.get('halt_reason'), 'OpenSpec evidence is insufficient')}.",
        "OpenSpec evidence is still expected before Grimoire resumes code or push actions.",
        "",
        "## Intended Work",
        f"Design plan artifact: `{plain(payload.get('plan_path'))}`.",
        "The fix stage must not guess at requirements or expand scope while this evidence gap exists.",
        "",
        "## Missing OpenSpec Evidence",
        *missing_lines,
        "",
        "## Suggested Spec Items",
        *gap_lines,
        "",
        quote_block(payload.get("suggested_spec_patch"), "Add OpenSpec requirements or scenarios for the missing in-scope behavior."),
        "",
        "## How To Rerun",
        "Add the missing OpenSpec evidence, update the PR, and let the normal pull_request synchronize event rerun Grimoire. This spec-gap note is advisory/non-blocking guidance, not a hard red failure; do not bypass the paused code-action guard with labels or local state.",
    ]
    return "\n".join(lines).rstrip() + "\n"


def write_json(path: pathlib.Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            text = "true" if value is True else "false" if value is False else str(value)
            handle.write(f"{key}={text}\n")


def run(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    input_path = resolve_path(args.input, workspace)
    comment_path = resolve_path(args.comment_output, workspace)
    status_path = resolve_path(args.status_output, workspace)
    try:
        payload = load_payload(input_path)
        sufficient = bool(payload["spec_sufficient"])
        if sufficient:
            comment = ""
            should_comment = False
            should_halt = False
        else:
            comment = render_comment(payload)
            section_count = sum(1 for line in comment.splitlines() if line.startswith("## "))
            if section_count != 5:
                raise ContractError(f"spec-gap comment must have exactly five top-level sections, got {section_count}")
            should_comment = True
            should_halt = True
        comment_path.parent.mkdir(parents=True, exist_ok=True)
        comment_path.write_text(comment, encoding="utf-8")
        status: dict[str, object] = {
            "schema_version": 1,
            "stage": STAGE,
            "generated_at": utc_now(),
            "status": "halt" if should_halt else "clear",
            "advisory": should_halt,
            "should_comment": should_comment,
            "should_halt": should_halt,
            "github_mutation_performed": False,
            "no_code_or_push_action": True,
            "comment_path": rel(comment_path, workspace),
            "source_path": rel(input_path, workspace),
            "top_level_sections": SECTIONS if should_comment else [],
            "top_level_section_count": 5 if should_comment else 0,
        }
        write_json(status_path, status)
        write_github_output(args.github_output, {"status": status["status"], "advisory": status["advisory"], "should_comment": should_comment, "should_halt": should_halt, "comment_path": str(comment_path), "status_path": str(status_path)})
        advisory_label = " advisory=true" if status["advisory"] is True else " advisory=false"
        print(f"{STAGE}: status={status['status']}{advisory_label} should_halt={str(should_halt).lower()} status_path={status_path}")
        return 0
    except ContractError as exc:
        status: dict[str, object] = {
            "schema_version": 1,
            "stage": STAGE,
            "generated_at": utc_now(),
            "status": "blocked",
            "advisory": False,
            "should_comment": False,
            "should_halt": True,
            "github_mutation_performed": False,
            "no_code_or_push_action": True,
            "blocked_reason": str(exc),
            "comment_path": rel(comment_path, workspace),
            "source_path": rel(input_path, workspace),
            "top_level_sections": [],
            "top_level_section_count": 0,
        }
        write_json(status_path, status)
        write_github_output(args.github_output, {"status": "blocked", "advisory": False, "should_comment": False, "should_halt": True, "status_path": str(status_path)})
        print(f"{STAGE}: blocked: {exc}", file=sys.stderr)
        return 1


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Render the Grimoire spec-gap advisory artifact.")
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--input", default=".omo/ci/spec-sufficiency.json")
    parser.add_argument("--comment-output", "--comment", dest="comment_output", default=".omo/ci/spec-gap-comment.md")
    parser.add_argument("--status-output", "--status", dest="status_output", default=".omo/ci/spec-gap-status.json")
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
