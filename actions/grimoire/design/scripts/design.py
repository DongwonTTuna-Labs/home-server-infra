#!/usr/bin/env python3
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import posixpath
import re
import sys
from datetime import datetime, timezone
from typing import cast

STAGE = "grimoire-design"
SUPPORTED_SPEC_SUFFIXES = {".md", ".txt", ".yaml", ".yml", ".json"}
REQUIRED_REVIEW_FIELDS = ("status", "approval_signal", "read_only", "mutation_allowed", "findings")
REQUIRED_FINDING_FIELDS = ("file", "line", "severity", "lens", "title", "what", "why", "suggested_fix", "evidence")


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


def norm_component(value: object) -> str:
    text = str(value or "").strip().lower().replace("\\", "/")
    text = posixpath.normpath(text) if text else ""
    if text == ".":
        text = ""
    text = re.sub(r"[^a-z0-9/._-]+", "-", text)
    text = re.sub(r"-+", "-", text).strip("-")
    return text or "unknown"


def fingerprint(repo: str, title: str, path: str) -> str:
    material = "|".join((norm_component(repo), norm_component(title), norm_component(path)))
    return hashlib.sha256(material.encode("utf-8")).hexdigest()[:24]


def load_review(path: pathlib.Path) -> dict[str, object]:
    if not path.exists():
        raise ContractError(f"review input does not exist: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"review input is not valid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise ContractError("review input must be a JSON object")
    for field in REQUIRED_REVIEW_FIELDS:
        if field not in payload:
            raise ContractError(f"review input missing root field: {field}")
    if payload.get("read_only") is not True:
        raise ContractError("review input read_only must be true")
    if payload.get("mutation_allowed") is not False:
        raise ContractError("review input mutation_allowed must be false")
    findings = payload.get("findings")
    if not isinstance(findings, list):
        raise ContractError("review findings must be an array")
    for index, finding in enumerate(findings):
        if not isinstance(finding, dict):
            raise ContractError(f"finding {index} must be an object")
        for field in REQUIRED_FINDING_FIELDS:
            if field not in finding:
                raise ContractError(f"finding {index} missing field: {field}")
    return payload


def is_spec_file(path: pathlib.Path) -> bool:
    return path.is_file() and path.suffix.lower() in SUPPORTED_SPEC_SUFFIXES


def collect_specs(workspace: pathlib.Path, spec_root: str, explicit: list[str]) -> tuple[list[pathlib.Path], list[str]]:
    spec_files: list[pathlib.Path] = []
    missing: list[str] = []
    for raw in explicit:
        path = resolve_path(raw, workspace)
        if not path.exists():
            missing.append(raw)
            continue
        if path.is_file() and is_spec_file(path):
            spec_files.append(path)
        elif path.is_dir():
            for child in sorted(path.rglob("*")):
                if "archive" in child.parts:
                    continue
                if is_spec_file(child):
                    spec_files.append(child)
    if not explicit:
        root = resolve_path(spec_root, workspace)
        for subdir in (root / "specs", root / "changes"):
            if not subdir.exists():
                continue
            for child in sorted(subdir.rglob("*")):
                if "archive" in child.parts:
                    continue
                if is_spec_file(child):
                    spec_files.append(child)
    seen: set[str] = set()
    unique: list[pathlib.Path] = []
    for path in spec_files:
        key = str(path.resolve())
        if key not in seen:
            unique.append(path)
            seen.add(key)
    return unique, missing


def spec_citation(path: pathlib.Path, workspace: pathlib.Path) -> dict[str, object]:
    try:
        lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    except OSError:
        lines = []
    for line_number, line in enumerate(lines, start=1):
        if line.strip():
            return {"path": rel(path, workspace), "line": line_number, "text": line.strip()}
    return {"path": rel(path, workspace), "line": 1, "text": "OpenSpec evidence file exists"}


def finding_location(finding: dict[str, object]) -> str:
    file_name = str(finding.get("file") or "unknown")
    line = finding.get("line")
    return f"{file_name}:{line}" if line not in (None, "") else file_name


def finding_is_out_of_scope(finding: dict[str, object]) -> bool:
    markers = [
        str(finding.get("scope") or ""),
        str(finding.get("classification") or ""),
        str(finding.get("disposition") or ""),
    ]
    title = str(finding.get("title") or "")
    return bool(finding.get("out_of_scope")) or any(value.lower().replace("-", "_") == "out_of_scope" for value in markers) or title.lower().startswith("[out-of-scope]")


def write_plan(path: pathlib.Path, payload: dict[str, object]) -> None:
    lines = [
        "# Grimoire Design Plan",
        "",
        f"Spec sufficient: `{str(payload['spec_sufficient']).lower()}`",
        f"Halt reason: {payload.get('halt_reason') or 'none'}",
        "",
        "## In Scope",
    ]
    in_scope = payload.get("in_scope")
    if isinstance(in_scope, list) and in_scope:
        for item in in_scope:
            if isinstance(item, dict):
                lines.append(f"- {item.get('location', 'unknown')}: {item.get('title', 'untitled')}")
    else:
        lines.append("- No in-scope findings require implementation.")
    lines.extend(["", "## Out Of Scope"])
    out_of_scope = payload.get("out_of_scope")
    if isinstance(out_of_scope, list) and out_of_scope:
        for item in out_of_scope:
            if isinstance(item, dict):
                lines.append(f"- {item.get('fingerprint')}: {item.get('title', 'untitled')} ({item.get('path', 'unknown')})")
    else:
        lines.append("- No out-of-scope findings were classified.")
    if not payload["spec_sufficient"]:
        lines.extend(["", "## Halt", "This is a halt-only artifact. The fix stage must not edit code from this plan."])
    else:
        lines.extend(["", "## Fix Boundary", "Later fix stages must not expand scope beyond PR-touched paths, declared test/docs/spec extras, and cited OpenSpec paths."])
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


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
    review_input = resolve_path(args.review_input, workspace)
    output = resolve_path(args.output, workspace)
    plan = resolve_path(args.plan, workspace)
    try:
        review = load_review(review_input)
        spec_files, missing_spec_inputs = collect_specs(workspace, args.spec_root, args.spec)
        in_scope: list[dict[str, object]] = []
        out_of_scope: list[dict[str, object]] = []
        bindings: list[dict[str, object]] = []
        missing: list[dict[str, object]] = []
        safety_default_gaps: list[dict[str, object]] = []
        citation = spec_citation(spec_files[0], workspace) if spec_files else None
        findings = cast(list[dict[str, object]], review["findings"])
        for index, raw_finding in enumerate(findings):
            finding = dict(raw_finding)  # type: ignore[arg-type]
            location = finding_location(finding)
            title = str(finding.get("title") or "untitled finding")
            path = str(finding.get("file") or "unknown")
            if finding_is_out_of_scope(finding):
                out_of_scope.append(
                    {
                        "finding_index": index,
                        "title": title,
                        "path": path,
                        "location": location,
                        "fingerprint": fingerprint(args.repository, title, path),
                        "repo": args.repository,
                        "suggested_owner": "repo-maintainer",
                        "issue_write_only": True,
                        "finding": finding,
                    }
                )
                continue
            scoped: dict[str, object] = {"finding_index": index, "title": title, "path": path, "location": location, "finding": finding}
            in_scope.append(scoped)
            if citation is None:
                missing_item: dict[str, object] = {
                    "finding_index": index,
                    "finding_file": path,
                    "finding_line": finding.get("line"),
                    "finding_title": title,
                    "location": location,
                    "reason": "No active OpenSpec evidence file was found for this in-scope review finding.",
                    "required_evidence": "Add an OpenSpec requirement or scenario that names the affected behavior before the fix stage runs.",
                    "suggested_spec_section": f"### Requirement: {title}\n- Binding key: {location}\n- Required behavior: describe the intended behavior.\n- Acceptance: cite an observable pass/fail condition.",
                }
                missing.append(missing_item)
                safety_default_gaps.append(
                    {
                        "scope": location,
                        "gap": "Review finding lacks an explicit OpenSpec binding.",
                        "required_default": "Halt before creating an executable fix plan for this finding.",
                    }
                )
            else:
                bindings.append(
                    {
                        "finding_index": index,
                        "finding_title": title,
                        "finding_location": location,
                        "requirement": citation["text"],
                        "citation": f"{citation['path']}:{citation['line']}",
                        "citations": [f"{citation['path']}:{citation['line']}"],
                        "evidence": citation["text"],
                    }
                )
        spec_sufficient = not missing and not missing_spec_inputs
        suggested_spec_patch = ""
        if missing or missing_spec_inputs:
            suggested = [item["suggested_spec_section"] for item in missing]
            if missing_spec_inputs:
                suggested.append("Missing explicit spec inputs: " + ", ".join(missing_spec_inputs))
            suggested_spec_patch = "\n\n".join(str(item) for item in suggested)
        payload: dict[str, object] = {
            "schema_version": 1,
            "stage": STAGE,
            "generated_at": utc_now(),
            "status": "sufficient" if spec_sufficient else "insufficient",
            "spec_sufficient": spec_sufficient,
            "should_halt": not spec_sufficient,
            "halt_reason": "" if spec_sufficient else "one or more in-scope review findings lack OpenSpec evidence",
            "scope_authority": "OpenSpec and OMO",
            "review_findings_count": len(findings),
            "in_scope": in_scope,
            "out_of_scope": out_of_scope,
            "out_of_scope_issue_write_only": True,
            "out_of_scope_dedup_key": "repo + normalized title/path sha256",
            "bindings": bindings,
            "missing": missing,
            "safety_default_gaps": safety_default_gaps,
            "suggested_spec_patch": suggested_spec_patch,
            "plan_path": rel(plan, workspace),
            "spec_evidence_paths": [rel(path, workspace) for path in spec_files],
            "missing_spec_inputs": missing_spec_inputs,
        }
        write_plan(plan, payload)
        write_json(output, payload)
        write_github_output(
            args.github_output,
            {
                "status": payload["status"],
                "spec_sufficient": spec_sufficient,
                "should_halt": not spec_sufficient,
                "output_path": str(output),
                "plan_path": str(plan),
                "in_scope_count": len(in_scope),
                "out_of_scope_count": len(out_of_scope),
            },
        )
        print(f"{STAGE}: status={payload['status']} in_scope={len(in_scope)} out_of_scope={len(out_of_scope)} output={output}")
        return 0 if spec_sufficient else 1
    except ContractError as exc:
        payload: dict[str, object] = {
            "schema_version": 1,
            "stage": STAGE,
            "generated_at": utc_now(),
            "status": "blocked",
            "spec_sufficient": False,
            "should_halt": True,
            "halt_reason": str(exc),
            "scope_authority": "OpenSpec and OMO",
            "in_scope": [],
            "out_of_scope": [],
            "bindings": [],
            "missing": [],
            "safety_default_gaps": [],
            "suggested_spec_patch": "",
            "plan_path": rel(plan, workspace),
        }
        write_plan(plan, payload)
        write_json(output, payload)
        write_github_output(args.github_output, {"status": "blocked", "spec_sufficient": False, "should_halt": True, "output_path": str(output), "plan_path": str(plan)})
        print(f"{STAGE}: blocked: {exc}", file=sys.stderr)
        return 1


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the Grimoire design/OpenSpec binding stage.")
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--review-input", "--input", dest="review_input", default=".omo/ci/review-findings.json")
    parser.add_argument("--output", default=".omo/ci/spec-sufficiency.json")
    parser.add_argument("--plan", default=".omo/ci/design-plan.md")
    parser.add_argument("--spec-root", default="openspec")
    parser.add_argument("--spec", action="append", default=[])
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", "local-consumer"))
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
