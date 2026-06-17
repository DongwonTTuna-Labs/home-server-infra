#!/usr/bin/env python3
from __future__ import annotations

import argparse
import fnmatch
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
TARGET_PATH_FIELDS = ("target_path", "target_paths", "allowed_write_path", "allowed_write_paths", "allowed_path", "allowed_paths")
REQUIRED_REVIEW_FIELDS = ("status", "approval_signal", "read_only", "mutation_allowed", "findings")
REQUIRED_FINDING_FIELDS = ("file", "line", "severity", "lens", "title", "what", "why", "suggested_fix", "evidence")
SEVERITY_ORDER = {"info": 0, "low": 1, "medium": 2, "high": 3, "critical": 4}
BINDING_HALT_THRESHOLD = "high"
HALTING_BINDING_SEVERITIES = frozenset(severity for severity, rank in SEVERITY_ORDER.items() if rank >= SEVERITY_ORDER[BINDING_HALT_THRESHOLD])
GATE_MODE = "severity-threshold"
SCOPE_MANIFEST_GATE_MODE = "scope-manifest"
SCOPE_MANIFEST_RELATIVE_PATH = ".omo/grimoire/scope.yml"
SCOPE_MANIFEST_KEYS = frozenset(("version", "schema_version", "governed_paths", "advisory_only_paths"))
SCOPE_MANIFEST_LIST_KEYS = frozenset(("governed_paths", "advisory_only_paths"))
SCOPE_GLOB_PATTERN = re.compile(r"^[A-Za-z0-9._/@*?\[\]!-]+$")


class ContractError(Exception):
    pass


class ScopeManifestProblem(Exception):
    def __init__(self, status: str, detail: str) -> None:
        super().__init__(detail)
        self.status: str = status
        self.detail: str = detail


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


def normalize_path(raw: object) -> str | None:
    text = str(raw or "").strip().replace("\\", "/")
    while text.startswith("./"):
        text = text[2:]
    if not text:
        return ""
    if text.startswith("/"):
        return None
    normalized = posixpath.normpath(text)
    if normalized == ".":
        return ""
    if ".." in pathlib.PurePosixPath(normalized).parts:
        return None
    return normalized


def sanitize_manifest_detail(raw: object) -> str:
    text = str(raw or "scope manifest could not be loaded").replace("\n", " ").replace("\r", " ")
    text = re.sub(r"[^A-Za-z0-9 ._:/@=+,-]+", "-", text)
    text = re.sub(r"\s+", " ", text).strip()
    return (text or "scope manifest could not be loaded")[:200]


def strip_yaml_comment(line: str) -> str:
    in_single = False
    in_double = False
    escaped = False
    for index, char in enumerate(line):
        if escaped:
            escaped = False
            continue
        if char == "\\" and in_double:
            escaped = True
            continue
        if char == "'" and not in_double:
            in_single = not in_single
            continue
        if char == '"' and not in_single:
            in_double = not in_double
            continue
        if char == "#" and not in_single and not in_double:
            return line[:index].rstrip()
    return line.rstrip()


def parse_yaml_scalar(raw: str) -> object:
    value = raw.strip()
    if value.startswith('"') and value.endswith('"'):
        try:
            parsed_scalar: object = json.loads(value)
            return parsed_scalar
        except json.JSONDecodeError as exc:
            raise ScopeManifestProblem("malformed", f"invalid quoted scalar at line {exc.lineno} column {exc.colno}") from exc
    if value.startswith("'") and value.endswith("'"):
        return value[1:-1].replace("''", "'")
    if re.fullmatch(r"[0-9]+", value):
        return int(value)
    return value


def parse_yaml_inline_list(raw: str) -> list[object]:
    value = raw.strip()
    if not value.endswith("]"):
        raise ScopeManifestProblem("malformed", "inline list is missing closing bracket")
    parsed: object
    try:
        parsed = json.loads(value)
    except json.JSONDecodeError:
        inner = value[1:-1].strip()
        if not inner:
            return []
        parsed = [parse_yaml_scalar(item.strip()) for item in inner.split(",")]
    if not isinstance(parsed, list):
        raise ScopeManifestProblem("malformed", "inline list must parse to an array")
    return cast(list[object], parsed)


def parse_simple_scope_yaml(text: str) -> dict[str, object]:
    payload: dict[str, object] = {}
    current_list_key = ""
    for line_number, raw_line in enumerate(text.splitlines(), start=1):
        line = strip_yaml_comment(raw_line)
        if not line.strip():
            continue
        if line.startswith((" ", "\t")):
            stripped = line.strip()
            if not current_list_key or not stripped.startswith("- "):
                raise ScopeManifestProblem("malformed", f"unsupported indentation at line {line_number}")
            value = parse_yaml_scalar(stripped[2:].strip())
            cast(list[object], payload[current_list_key]).append(value)
            continue
        current_list_key = ""
        if ":" not in line:
            raise ScopeManifestProblem("malformed", f"expected key-value pair at line {line_number}")
        raw_key, raw_value = line.split(":", 1)
        key = raw_key.strip()
        value = raw_value.strip()
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key):
            raise ScopeManifestProblem("malformed", f"invalid key at line {line_number}")
        if key in payload:
            raise ScopeManifestProblem("malformed", f"duplicate key at line {line_number}")
        if value == "":
            payload[key] = []
            current_list_key = key
        elif value.startswith("["):
            payload[key] = parse_yaml_inline_list(value)
        else:
            payload[key] = parse_yaml_scalar(value)
    return payload


def parse_scope_manifest_text(text: str) -> dict[str, object]:
    stripped = text.strip()
    if not stripped:
        raise ScopeManifestProblem("invalid", "scope manifest is empty")
    if stripped.startswith("{"):
        try:
            parsed: object = json.loads(text)
        except json.JSONDecodeError as exc:
            raise ScopeManifestProblem("malformed", f"JSON parse error at line {exc.lineno} column {exc.colno}") from exc
        if not isinstance(parsed, dict):
            raise ScopeManifestProblem("invalid", "scope manifest root must be an object")
        return cast(dict[str, object], parsed)
    return parse_simple_scope_yaml(text)


def normalize_scope_glob(raw: object, field: str, index: int) -> str:
    if not isinstance(raw, str):
        raise ScopeManifestProblem("invalid", f"{field}[{index}] must be a string")
    if "\\" in raw:
        raise ScopeManifestProblem("invalid", f"{field}[{index}] must use forward slashes")
    if raw != raw.strip():
        raise ScopeManifestProblem("invalid", f"{field}[{index}] must not contain surrounding whitespace")
    pattern = raw
    while pattern.startswith("./"):
        pattern = pattern[2:]
    if not pattern or pattern == ".":
        raise ScopeManifestProblem("invalid", f"{field}[{index}] must not be empty")
    if len(pattern) > 255:
        raise ScopeManifestProblem("invalid", f"{field}[{index}] is too long")
    if pattern.startswith("/") or pattern.startswith("~") or "//" in pattern:
        raise ScopeManifestProblem("invalid", f"{field}[{index}] must be a safe relative glob")
    if ".." in pathlib.PurePosixPath(pattern).parts:
        raise ScopeManifestProblem("invalid", f"{field}[{index}] must not contain parent traversal")
    if not SCOPE_GLOB_PATTERN.fullmatch(pattern):
        raise ScopeManifestProblem("invalid", f"{field}[{index}] contains unsupported characters")
    return pattern.rstrip("/") or pattern


def scope_glob_list(payload: dict[str, object], field: str, required: bool) -> list[str]:
    raw = payload.get(field)
    if raw is None:
        if required:
            raise ScopeManifestProblem("invalid", f"{field} is required")
        return []
    if not isinstance(raw, list):
        raise ScopeManifestProblem("invalid", f"{field} must be a list")
    raw_values = cast(list[object], raw)
    values: list[str] = []
    seen: set[str] = set()
    for index, value in enumerate(raw_values):
        pattern = normalize_scope_glob(value, field, index)
        if pattern in seen:
            raise ScopeManifestProblem("invalid", f"{field}[{index}] duplicates an earlier glob")
        values.append(pattern)
        seen.add(pattern)
    if required and not values:
        raise ScopeManifestProblem("invalid", f"{field} must contain at least one glob")
    return values


def validate_scope_manifest(payload: dict[str, object]) -> dict[str, object]:
    unknown_keys = sorted(set(payload) - SCOPE_MANIFEST_KEYS)
    if unknown_keys:
        raise ScopeManifestProblem("invalid", "unsupported keys: " + ", ".join(unknown_keys))
    version = payload.get("schema_version", payload.get("version"))
    if "version" in payload and "schema_version" in payload and payload["version"] != payload["schema_version"]:
        raise ScopeManifestProblem("invalid", "version and schema_version must match")
    if version not in (1, "1"):
        raise ScopeManifestProblem("invalid", "version or schema_version must be 1")
    governed_paths = scope_glob_list(payload, "governed_paths", required=True)
    advisory_only_paths = scope_glob_list(payload, "advisory_only_paths", required=False)
    return {
        "version": 1,
        "governed_paths": governed_paths,
        "advisory_only_paths": advisory_only_paths,
    }


def empty_scope_manifest(status: str, detail: str = "") -> dict[str, object]:
    diagnostics = [sanitize_manifest_detail(detail)] if detail else []
    return {
        "status": status,
        "path": SCOPE_MANIFEST_RELATIVE_PATH,
        "version": None,
        "governed_paths": [],
        "advisory_only_paths": [],
        "diagnostics": diagnostics,
    }


def load_scope_manifest(workspace: pathlib.Path) -> dict[str, object]:
    manifest_path = workspace / SCOPE_MANIFEST_RELATIVE_PATH
    if not manifest_path.exists():
        return empty_scope_manifest("absent")
    try:
        parsed = parse_scope_manifest_text(manifest_path.read_text(encoding="utf-8"))
        validated = validate_scope_manifest(parsed)
    except OSError as exc:
        return empty_scope_manifest("malformed", f"scope manifest could not be read: {exc.__class__.__name__}")
    except UnicodeError as exc:
        return empty_scope_manifest("malformed", f"scope manifest is not valid UTF-8: {exc.__class__.__name__}")
    except ScopeManifestProblem as exc:
        return empty_scope_manifest(exc.status, exc.detail)
    return {
        "status": "loaded",
        "path": SCOPE_MANIFEST_RELATIVE_PATH,
        "version": validated["version"],
        "governed_paths": validated["governed_paths"],
        "advisory_only_paths": validated["advisory_only_paths"],
        "diagnostics": [],
    }


def scope_manifest_gate_mode(scope_manifest: dict[str, object]) -> str:
    return SCOPE_MANIFEST_GATE_MODE if scope_manifest.get("status") == "loaded" else GATE_MODE


def path_matches_scope_glob(path: str, pattern: str) -> bool:
    if fnmatch.fnmatchcase(path, pattern):
        return True
    if pattern.endswith("/**"):
        prefix = pattern[:-3].rstrip("/")
        return path == prefix or path.startswith(prefix + "/")
    if not any(char in pattern for char in "*?["):
        return path == pattern or path.startswith(pattern + "/")
    return False


def first_scope_glob_match(path: str, patterns: list[str]) -> str:
    for pattern in patterns:
        if path_matches_scope_glob(path, pattern):
            return pattern
    return ""


def scope_manifest_classification(scope_manifest: dict[str, object], raw_path: str) -> tuple[str, str]:
    if scope_manifest.get("status") != "loaded":
        return "", ""
    normalized = normalize_path(raw_path)
    if not normalized:
        return "ungoverned", ""
    governed_match = first_scope_glob_match(normalized, cast(list[str], scope_manifest.get("governed_paths", [])))
    if governed_match:
        return "governed", governed_match
    advisory_match = first_scope_glob_match(normalized, cast(list[str], scope_manifest.get("advisory_only_paths", [])))
    if advisory_match:
        return "advisory_only", advisory_match
    return "ungoverned", ""


def scope_manifest_payload_fields(scope_manifest: dict[str, object]) -> dict[str, object]:
    return {
        "manifest_status": scope_manifest["status"],
        "manifest_path": scope_manifest["path"],
        "manifest_diagnostics": scope_manifest["diagnostics"],
        "scope_manifest": {
            "version": scope_manifest["version"],
            "governed_paths": scope_manifest["governed_paths"],
            "advisory_only_paths": scope_manifest["advisory_only_paths"],
        },
    }


def direct_extra_allowed(path: str) -> bool:
    if path.startswith(("tests/", "docs/", "openspec/", "spec/", "schemas/")):
        return True
    name = pathlib.PurePosixPath(path).name
    return name.endswith(("_test.py", "_test.rs", ".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx"))


def append_target_path(raw: object, paths: list[str], seen: set[str]) -> list[str]:
    normalized = normalize_path(raw)
    if normalized and direct_extra_allowed(normalized) and normalized not in seen:
        paths.append(normalized)
        seen.add(normalized)
        return [normalized]
    return []


def collect_finding_target_values(finding: dict[str, object]) -> tuple[bool, list[object]]:
    raw_values: list[object] = []
    explicit = False
    for field in TARGET_PATH_FIELDS:
        if field not in finding:
            continue
        explicit = True
        value = finding.get(field)
        if isinstance(value, list):
            raw_values.extend(cast(list[object], value))
        elif value:
            raw_values.append(value)
    return explicit, raw_values


def valid_and_invalid_target_paths(raw_values: list[object]) -> tuple[list[str], list[str]]:
    valid: list[str] = []
    invalid: list[str] = []
    for value in raw_values:
        raw = str(value or "").strip()
        normalized = normalize_path(raw)
        if normalized and direct_extra_allowed(normalized):
            valid.append(normalized)
        elif raw:
            invalid.append(raw)
    return valid, sorted(set(invalid))


def append_valid_target_paths(valid_paths: list[str], paths: list[str], seen: set[str]) -> list[str]:
    added_paths: list[str] = []
    for path in valid_paths:
        if path not in seen:
            paths.append(path)
            seen.add(path)
            added_paths.append(path)
    return added_paths


def extract_finding_target_paths(finding: dict[str, object], paths: list[str], seen: set[str]) -> tuple[bool, list[str], list[str]]:
    explicit, raw_values = collect_finding_target_values(finding)
    valid, invalid = valid_and_invalid_target_paths(raw_values)
    return explicit, append_valid_target_paths(valid, paths, seen), invalid


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


def normalize_severity(raw: object, index: int) -> str:
    severity = str(raw or "").strip().lower()
    if severity not in SEVERITY_ORDER:
        raise ContractError(f"finding {index} has unsupported severity: {raw!r}")
    return severity


def missing_binding_requires_halt(severity: str, scope_manifest_requires_binding: bool = False) -> bool:
    return scope_manifest_requires_binding or severity in HALTING_BINDING_SEVERITIES


def suggested_spec_section(title: str, location: str) -> str:
    return f"### Requirement: {title}\n- Binding key: {location}\n- Required behavior: describe the intended behavior.\n- Acceptance: cite an observable pass/fail condition."


def missing_binding_entry(index: int, finding: dict[str, object], path: str, title: str, location: str, severity: str, advisory: bool, gate_mode: str) -> dict[str, object]:
    if advisory:
        reason = f"No active OpenSpec evidence file was found for this lower-severity in-scope review finding; the {gate_mode} gate records it as advisory under the high/critical halt threshold."
        required_evidence = "Add an OpenSpec requirement or scenario before raising this finding to high/critical severity."
    else:
        reason = "No active OpenSpec evidence file was found for this in-scope review finding."
        required_evidence = "Add an OpenSpec requirement or scenario that names the affected behavior before the fix stage runs."
    return {
        "finding_index": index,
        "finding_file": path,
        "finding_line": finding.get("line"),
        "finding_title": title,
        "location": location,
        "severity": severity,
        "gate_mode": gate_mode,
        "halt_threshold": BINDING_HALT_THRESHOLD,
        "reason": reason,
        "required_evidence": required_evidence,
        "suggested_spec_section": suggested_spec_section(title, location),
    }


def finding_is_out_of_scope(finding: dict[str, object]) -> bool:
    markers = [
        str(finding.get("scope") or ""),
        str(finding.get("classification") or ""),
        str(finding.get("disposition") or ""),
    ]
    title = str(finding.get("title") or "")
    return bool(finding.get("out_of_scope")) or any(value.lower().replace("-", "_") == "out_of_scope" for value in markers) or title.lower().startswith("[out-of-scope]")


def out_of_scope_entry(
    index: int,
    finding: dict[str, object],
    title: str,
    path: str,
    location: str,
    repository: str,
    reason: str,
    manifest_classification: str = "",
    manifest_match: str = "",
    invalid_target_paths: list[str] | None = None,
) -> dict[str, object]:
    entry: dict[str, object] = {
        "finding_index": index,
        "title": title,
        "path": path,
        "location": location,
        "fingerprint": fingerprint(repository, title, path),
        "repo": repository,
        "suggested_owner": "repo-maintainer",
        "issue_write_only": True,
        "out_of_scope_reason": reason,
        "finding": finding,
    }
    if manifest_classification:
        entry["scope_manifest_classification"] = manifest_classification
    if manifest_match:
        entry["scope_manifest_matched_glob"] = manifest_match
    if invalid_target_paths:
        entry["invalid_target_paths"] = invalid_target_paths
    return entry


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
    advisory_gaps = payload.get("advisory_gaps")
    if isinstance(advisory_gaps, list) and advisory_gaps:
        lines.extend(["", "## Advisory Gaps"])
        for item in advisory_gaps:
            if isinstance(item, dict):
                lines.append(f"- {item.get('location', 'unknown')}: {item.get('reason', 'advisory gap')}")
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
    scope_manifest = load_scope_manifest(workspace)
    gate_mode = scope_manifest_gate_mode(scope_manifest)
    try:
        review = load_review(review_input)
        spec_files, missing_spec_inputs = collect_specs(workspace, args.spec_root, args.spec)
        in_scope: list[dict[str, object]] = []
        out_of_scope: list[dict[str, object]] = []
        bindings: list[dict[str, object]] = []
        missing: list[dict[str, object]] = []
        advisory_gaps: list[dict[str, object]] = []
        safety_default_gaps: list[dict[str, object]] = []
        invalid_allowed_write_paths: list[str] = []
        citation = spec_citation(spec_files[0], workspace) if spec_files else None
        target_paths: list[str] = []
        target_seen: set[str] = set()
        findings = cast(list[dict[str, object]], review["findings"])
        for index, raw_finding in enumerate(findings):
            finding = dict(raw_finding)  # type: ignore[arg-type]
            location = finding_location(finding)
            title = str(finding.get("title") or "untitled finding")
            path = str(finding.get("file") or "unknown")
            severity = normalize_severity(finding.get("severity"), index)
            manifest_classification, manifest_match = scope_manifest_classification(scope_manifest, path)
            has_explicit_targets, target_values = collect_finding_target_values(finding)
            valid_finding_targets, invalid_finding_targets = valid_and_invalid_target_paths(target_values)
            if invalid_finding_targets:
                invalid_allowed_write_paths.extend(invalid_finding_targets)
                safety_default_gaps.append(
                    {
                        "scope": location,
                        "gap": "Review finding declared unsafe target paths.",
                        "invalid_target_paths": invalid_finding_targets,
                        "required_default": "Halt before authorizing fix writes for this finding.",
                    }
                )
            review_out_of_scope = finding_is_out_of_scope(finding)
            manifest_out_of_scope_reason = ""
            if manifest_classification == "advisory_only":
                manifest_out_of_scope_reason = "scope-manifest-advisory-only"
            elif manifest_classification == "ungoverned":
                manifest_out_of_scope_reason = "scope-manifest-ungoverned"
            if review_out_of_scope or manifest_out_of_scope_reason:
                out_of_scope.append(
                    out_of_scope_entry(
                        index,
                        finding,
                        title,
                        path,
                        location,
                        args.repository,
                        "review-classified-out-of-scope" if review_out_of_scope else manifest_out_of_scope_reason,
                        manifest_classification,
                        manifest_match,
                        invalid_finding_targets,
                    )
                )
                continue
            scoped: dict[str, object] = {"finding_index": index, "title": title, "path": path, "location": location, "severity": severity, "finding": finding}
            if manifest_classification:
                scoped["scope_manifest_classification"] = manifest_classification
            if manifest_match:
                scoped["scope_manifest_matched_glob"] = manifest_match
            if has_explicit_targets:
                finding_target_paths = append_valid_target_paths(valid_finding_targets, target_paths, target_seen)
            else:
                finding_target_paths = append_target_path(path, target_paths, target_seen)
            if finding_target_paths:
                scoped["target_paths"] = finding_target_paths
            if invalid_finding_targets:
                scoped["invalid_target_paths"] = invalid_finding_targets
            in_scope.append(scoped)
            if citation is None:
                requires_binding_halt = missing_binding_requires_halt(severity)
                missing_item = missing_binding_entry(index, finding, path, title, location, severity, advisory=not requires_binding_halt, gate_mode=gate_mode)
                if requires_binding_halt:
                    missing.append(missing_item)
                    safety_default_gaps.append(
                        {
                            "scope": location,
                            "severity": severity,
                            "gate_mode": gate_mode,
                            "gap": "Review finding lacks an explicit OpenSpec binding.",
                            "required_default": "Halt before creating an executable fix plan for this high-severity finding.",
                        }
                    )
                else:
                    advisory_gaps.append(missing_item)
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
                        "target_paths": finding_target_paths,
                        "invalid_target_paths": invalid_finding_targets,
                    }
                )
        invalid_allowed_write_paths = sorted(set(invalid_allowed_write_paths))
        halt_reasons: list[str] = []
        if missing:
            halt_reasons.append("one or more high-severity in-scope review findings lack OpenSpec evidence")
        if missing_spec_inputs:
            halt_reasons.append("one or more explicit OpenSpec inputs were missing")
        if invalid_allowed_write_paths:
            halt_reasons.append("one or more review findings declared unsafe target paths")
        spec_sufficient = not halt_reasons
        halt_reason = "" if spec_sufficient else "; ".join(halt_reasons)
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
            "halt_reason": halt_reason,
            "gate_mode": gate_mode,
            **scope_manifest_payload_fields(scope_manifest),
            "scope_authority": "OpenSpec and OMO",
            "review_findings_count": len(findings),
            "in_scope": in_scope,
            "out_of_scope": out_of_scope,
            "out_of_scope_issue_write_only": True,
            "out_of_scope_dedup_key": "repo + normalized title/path sha256",
            "bindings": bindings,
            "target_paths": target_paths,
            "allowed_write_paths": target_paths,
            "invalid_allowed_write_paths": invalid_allowed_write_paths,
            "missing": missing,
            "advisory_gaps": advisory_gaps,
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
                "manifest_status": scope_manifest["status"],
                "gate_mode": gate_mode,
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
            "gate_mode": gate_mode,
            **scope_manifest_payload_fields(scope_manifest),
            "scope_authority": "OpenSpec and OMO",
            "in_scope": [],
            "out_of_scope": [],
            "bindings": [],
            "target_paths": [],
            "allowed_write_paths": [],
            "invalid_allowed_write_paths": [],
            "missing": [],
            "advisory_gaps": [],
            "safety_default_gaps": [],
            "suggested_spec_patch": "",
            "plan_path": rel(plan, workspace),
        }
        write_plan(plan, payload)
        write_json(output, payload)
        write_github_output(
            args.github_output,
            {
                "status": "blocked",
                "spec_sufficient": False,
                "should_halt": True,
                "output_path": str(output),
                "plan_path": str(plan),
                "manifest_status": scope_manifest["status"],
                "gate_mode": gate_mode,
            },
        )
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
