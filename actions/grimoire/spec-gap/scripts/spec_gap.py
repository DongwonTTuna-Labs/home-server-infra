#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import sys
from datetime import datetime, timezone
from typing import cast

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
        raw_payload = cast(object, json.loads(path.read_text(encoding="utf-8")))
    except json.JSONDecodeError as exc:
        raise ContractError(f"input is not valid JSON: {exc}") from exc
    if not isinstance(raw_payload, dict):
        raise ContractError("design input must be a JSON object")
    payload = cast(dict[str, object], raw_payload)
    for field in REQUIRED_FIELDS:
        if field not in payload:
            raise ContractError(f"design input missing root field: {field}")
    if not isinstance(payload.get("spec_sufficient"), bool):
        raise ContractError("spec_sufficient must be boolean")
    for field in ("bindings", "missing", "safety_default_gaps"):
        if not isinstance(payload.get(field), list):
            raise ContractError(f"{field} must be an array")
    return payload


def list_items(value: object) -> list[object]:
    if isinstance(value, list):
        return cast(list[object], value)
    return []


def dict_items(value: object) -> list[dict[str, object]]:
    return [item for item in list_items(value) if isinstance(item, dict)]


def text_value(value: object, fallback: str = "") -> str:
    text = re.sub(r"\s+", " ", str(value if value is not None else "").strip())
    return text or fallback


def item_text(item: dict[str, object], keys: tuple[str, ...], fallback: str = "not provided") -> str:
    for key in keys:
        text = text_value(item.get(key))
        if text:
            return text
    return fallback


def string_items(value: object) -> list[str]:
    if isinstance(value, list):
        items = cast(list[object], value)
        return [text for text in (text_value(item) for item in items) if text]
    text = text_value(value)
    return [text] if text else []


def unique_strings(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        text = text_value(value)
        if text and text not in seen:
            result.append(text)
            seen.add(text)
    return result


def code_span(value: str) -> str:
    return "`" + value.replace("`", "'") + "`"


def code_list(values: list[str], fallback: str = "not provided") -> str:
    concrete = unique_strings(values)
    if not concrete:
        return fallback
    return ", ".join(code_span(value) for value in concrete)


def scoped_entries(payload: dict[str, object]) -> tuple[dict[str, dict[str, object]], dict[str, dict[str, object]]]:
    by_index: dict[str, dict[str, object]] = {}
    by_location: dict[str, dict[str, object]] = {}
    for item in dict_items(payload.get("in_scope")):
        index = text_value(item.get("finding_index"))
        location = text_value(item.get("location"))
        if index:
            by_index[index] = item
        if location:
            by_location[location] = item
    return by_index, by_location


def scoped_for_missing(item: dict[str, object], by_index: dict[str, dict[str, object]], by_location: dict[str, dict[str, object]]) -> dict[str, object]:
    index = text_value(item.get("finding_index"))
    location = text_value(item.get("location"))
    if index and index in by_index:
        return by_index[index]
    if location and location in by_location:
        return by_location[location]
    return {}


def concrete_payload_paths(payload: dict[str, object]) -> list[str]:
    paths: list[str] = []
    for key in ("target_paths", "allowed_write_paths", "spec_evidence_paths", "missing_spec_inputs"):
        paths.extend(string_items(payload.get(key)))
    for item in dict_items(payload.get("missing")):
        paths.extend(string_items(item.get("target_paths")))
        for key in ("finding_file", "path", "file", "location"):
            text = text_value(item.get(key))
            if text:
                paths.append(text)
                break
    for item in dict_items(payload.get("in_scope")):
        paths.extend(string_items(item.get("target_paths")))
        for key in ("path", "file", "location"):
            text = text_value(item.get(key))
            if text:
                paths.append(text)
                break
    for item in dict_items(payload.get("safety_default_gaps")):
        text = text_value(item.get("scope"))
        if text:
            paths.append(text)
    return unique_strings(paths)


def missing_evidence_records(payload: dict[str, object]) -> list[dict[str, object]]:
    by_index, by_location = scoped_entries(payload)
    records: list[dict[str, object]] = []
    for item in dict_items(payload.get("missing")):
        scoped = scoped_for_missing(item, by_index, by_location)
        location = item_text(item, ("location", "finding_file", "path", "file"))
        path = item_text(item, ("finding_file", "path", "file"), text_value(scoped.get("path"), location))
        title = item_text(item, ("finding_title", "title"), text_value(scoped.get("title"), "Missing OpenSpec evidence"))
        target_paths = unique_strings(string_items(item.get("target_paths")) + string_items(scoped.get("target_paths")) + [path])
        records.append(
            {
                "location": location,
                "path": path,
                "title": title,
                "severity": item_text(item, ("severity",), text_value(scoped.get("severity"), "unspecified severity")),
                "reason": item_text(item, ("reason",), "No active OpenSpec evidence was found."),
                "required_evidence": item_text(item, ("required_evidence",), "Add an OpenSpec requirement or scenario that names the affected behavior."),
                "target_paths": target_paths,
                "suggested_spec_section": text_value(item.get("suggested_spec_section")),
            }
        )
    if records:
        return records
    fallback_paths = concrete_payload_paths(payload)
    if fallback_paths:
        primary_path = fallback_paths[0]
        return [
            {
                "location": primary_path,
                "path": primary_path,
                "title": "Missing OpenSpec evidence for changed area",
                "severity": "unspecified severity",
                "reason": "The design payload did not include detailed missing entries, but it did name changed areas that still need evidence.",
                "required_evidence": "Add an OpenSpec requirement or scenario that names the affected behavior.",
                "target_paths": fallback_paths,
                "suggested_spec_section": "",
            }
        ]
    return []


def missing_evidence_lines(payload: dict[str, object], records: list[dict[str, object]]) -> list[str]:
    lines: list[str] = []
    for record in records:
        location = text_value(record.get("location"), "unknown location")
        title = text_value(record.get("title"), "Missing OpenSpec evidence")
        severity = text_value(record.get("severity"), "unspecified severity")
        target_paths = string_items(record.get("target_paths"))
        reason = text_value(record.get("reason"), "No active OpenSpec evidence was found.")
        required_evidence = text_value(record.get("required_evidence"), "Add an OpenSpec requirement or scenario that names the affected behavior.")
        lines.append(f"- {code_span(location)} ({severity}) affects {code_list(target_paths, code_span(location))}: {title}.")
        lines.append(f"  Missing because: {reason}")
        lines.append(f"  Needed evidence: {required_evidence}")
    for missing_input in string_items(payload.get("missing_spec_inputs")):
        lines.append(f"- Explicit OpenSpec input was requested but not found: {code_span(missing_input)}.")
    if not lines:
        lines.append("- No concrete missing evidence entries were provided; inspect the design plan before authorizing code action.")
    gaps = dict_items(payload.get("safety_default_gaps"))
    if gaps:
        lines.extend(["", "Safety defaults still preventing code/push action:"])
        for item in gaps:
            scope = item_text(item, ("scope", "location", "path"))
            required_default = item_text(item, ("required_default", "gap"), "Halt before authorizing fix writes.")
            lines.append(f"- {code_span(scope)}: {required_default}")
    return lines


def extract_change_id(payload: dict[str, object]) -> str:
    for key in ("change_id", "openspec_change_id", "spec_change_id", "change_slug", "change"):
        text = text_value(payload.get(key))
        if text:
            return text
    sources = string_items(payload.get("spec_evidence_paths")) + string_items(payload.get("plan_path")) + string_items(payload.get("suggested_spec_patch"))
    for source in sources:
        match = re.search(r"(?:^|/)openspec/changes/([^/\s`]+)/", source) or re.search(r"(?:^|/)changes/([^/\s`]+)/", source)
        if match:
            return match.group(1)
    return ""


def requirement_name(record: dict[str, object]) -> str:
    title = text_value(record.get("title"), "OpenSpec evidence")
    location = text_value(record.get("location"), text_value(record.get("path"), "changed area"))
    return f"Spec evidence for {title} at {location}"


def render_spec_skeleton(payload: dict[str, object], records: list[dict[str, object]]) -> str:
    concrete_records = records or missing_evidence_records(payload)
    if not concrete_records:
        fallback_paths = concrete_payload_paths(payload)
        fallback_path = fallback_paths[0] if fallback_paths else "the changed area"
        fallback_record: dict[str, object] = {
            "location": fallback_path,
            "path": fallback_path,
            "title": "changed behavior",
            "target_paths": [fallback_path],
        }
        concrete_records = [fallback_record]
    change_id = extract_change_id(payload)
    change_reference = f"OpenSpec change `{change_id}`" if change_id else "the active OpenSpec change for this PR"
    lines: list[str] = []
    for record in concrete_records:
        location = text_value(record.get("location"), "changed area")
        title = text_value(record.get("title"), "changed behavior")
        paths = string_items(record.get("target_paths")) or [text_value(record.get("path"), location)]
        lines.extend(
            [
                f"### Requirement: {requirement_name(record)}",
                f"{change_reference} SHALL describe the intended behavior for {code_list(paths, code_span(location))} before Grimoire performs code or push actions.",
                "",
                f"#### Scenario: Grimoire binds {location} to OpenSpec evidence",
                f"- **GIVEN** this PR affects {code_list(paths, code_span(location))}",
                "- **WHEN** Grimoire reruns after the OpenSpec evidence is pushed",
                f"- **THEN** the design stage finds an active requirement or scenario for {code_span(location)} and no longer reports `{title}` as missing OpenSpec evidence",
                "",
            ]
        )
    return "\n".join(lines).rstrip()


def render_comment(payload: dict[str, object]) -> str:
    records = missing_evidence_records(payload)
    concrete_paths = concrete_payload_paths(payload)
    concrete_path_text = code_list(concrete_paths, "the in-scope changed area named by the design payload")
    design_patch = str(payload.get("suggested_spec_patch") or "").rstrip()
    suggested_lines = [
        "Paste this skeleton under the active OpenSpec change/spec file, then tighten the requirement wording to the exact intended behavior:",
        "",
        "```markdown",
        render_spec_skeleton(payload, records),
        "```",
    ]
    if design_patch:
        suggested_lines.extend(["", "Design-suggested text retained for context:", quote_block(design_patch, "Add OpenSpec requirements or scenarios for the missing in-scope behavior.")])
    lines = [
        "<!-- grimoire-spec-gap -->",
        "## Summary",
        f"Grimoire paused automated code changes and recorded advisory, non-blocking guidance because {plain(payload.get('halt_reason'), 'OpenSpec evidence is insufficient')}.",
        "OpenSpec evidence is still expected before Grimoire resumes code or push actions; this advisory does not claim the spec is no longer required.",
        "",
        "## Intended Work",
        f"Design plan artifact: `{plain(payload.get('plan_path'))}`.",
        f"Affected files/areas from the design payload: {concrete_path_text}.",
        "The fix stage must not guess at requirements or expand scope while this evidence gap exists.",
        "",
        "### What To Modify vs Add",
        f"- Modify existing OpenSpec evidence if it already describes {concrete_path_text} but does not bind the current finding, intended behavior, or acceptance scenario.",
        f"- Add a new OpenSpec requirement/scenario when no active evidence covers {concrete_path_text}; keep the new evidence limited to those paths/areas.",
        "",
        "## Missing OpenSpec Evidence",
        *missing_evidence_lines(payload, records),
        "",
        "## Suggested Spec Items",
        *suggested_lines,
        "",
        "## How To Rerun",
        "This spec-gap note is advisory/non-blocking guidance, not a hard red failure, but it keeps Grimoire from taking code or push actions until truthful OpenSpec evidence exists.",
        "",
        "### How To Clear",
        "- Add or update the OpenSpec evidence above, push it to the PR branch, and let the next `pull_request.synchronize` event rerun Grimoire.",
        "- If the evidence already exists and a human wants to force re-review, remove the `📋 Spec Needed` label; the rerun must still validate the OpenSpec evidence before code action resumes.",
    ]
    return "\n".join(lines).rstrip() + "\n"


def write_json(path: pathlib.Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    _ = path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            text = "true" if value is True else "false" if value is False else str(value)
            _ = handle.write(f"{key}={text}\n")


def run(args: argparse.Namespace) -> int:
    consumer_workspace = cast(str, args.consumer_workspace)
    input_arg = cast(str, args.input)
    comment_output = cast(str, args.comment_output)
    status_output = cast(str, args.status_output)
    github_output = cast(str, args.github_output)
    workspace = pathlib.Path(consumer_workspace).resolve()
    input_path = resolve_path(input_arg, workspace)
    comment_path = resolve_path(comment_output, workspace)
    status_path = resolve_path(status_output, workspace)
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
        _ = comment_path.write_text(comment, encoding="utf-8")
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
        write_github_output(github_output, {"status": status["status"], "advisory": status["advisory"], "should_comment": should_comment, "should_halt": should_halt, "comment_path": str(comment_path), "status_path": str(status_path)})
        advisory_label = " advisory=true" if status["advisory"] is True else " advisory=false"
        print(f"{STAGE}: status={status['status']}{advisory_label} should_halt={str(should_halt).lower()} status_path={status_path}")
        return 0
    except ContractError as exc:
        blocked_status: dict[str, object] = {
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
        write_json(status_path, blocked_status)
        write_github_output(github_output, {"status": "blocked", "advisory": False, "should_comment": False, "should_halt": True, "status_path": str(status_path)})
        print(f"{STAGE}: blocked: {exc}", file=sys.stderr)
        return 1


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Render the Grimoire spec-gap advisory artifact.")
    _ = parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    _ = parser.add_argument("--input", default=".omo/ci/spec-sufficiency.json")
    _ = parser.add_argument("--comment-output", "--comment", dest="comment_output", default=".omo/ci/spec-gap-comment.md")
    _ = parser.add_argument("--status-output", "--status", dest="status_output", default=".omo/ci/spec-gap-status.json")
    _ = parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
