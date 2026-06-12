#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import posixpath
import sys
from datetime import datetime, timezone

STAGE = "grimoire-trusted-controller"
PROTECTED_DOCS = {
    "docs/SECURITY.md",
    "docs/REVIEW_CHECKLIST.md",
    "docs/FORKED_RELAYER_CRATE.md",
    "docs/PUBLISHING_DISABLED.md",
}
STAGES = (
    "trusted-controller",
    "review",
    "design",
    "spec-gap",
    "fix",
    "verify",
    "labels",
    "cast",
)


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def normalize_path(raw: str) -> str | None:
    text = str(raw).strip().replace("\\", "/")
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


def protected_reason(path: str) -> str:
    if path == ".github" or path.startswith(".github/"):
        return ".github/** controller or workflow path"
    if path == ".opencode" or path.startswith(".opencode/"):
        return ".opencode/** model/controller config path"
    if path == "opencode.json":
        return "opencode.json model/controller config path"
    if pathlib.PurePosixPath(path).name == "AGENTS.md":
        return "root or nested AGENTS.md instruction path"
    if path in PROTECTED_DOCS:
        return "security-critical documentation path"
    return ""


def read_changed(paths: list[str], single_paths: list[str]) -> tuple[list[str], list[str], list[dict[str, str]]]:
    raw_values: list[str] = []
    invalid: list[str] = []
    sources: list[dict[str, str]] = []
    for list_path in paths:
        source = pathlib.Path(list_path)
        if not source.exists():
            invalid.append(list_path)
            continue
        raw_values.extend(source.read_text(encoding="utf-8", errors="replace").splitlines())
        sources.append({"type": "file", "value": list_path})
    if os.environ.get("GRIMOIRE_CHANGED_FILES"):
        raw_values.extend(os.environ["GRIMOIRE_CHANGED_FILES"].splitlines())
        sources.append({"type": "env", "value": "GRIMOIRE_CHANGED_FILES"})
    raw_values.extend(single_paths)
    if single_paths:
        sources.append({"type": "argv", "value": "--changed-file"})

    changed: list[str] = []
    seen: set[str] = set()
    for raw in raw_values:
        normalized = normalize_path(raw)
        if normalized is None:
            invalid.append(str(raw))
            continue
        if normalized and normalized not in seen:
            changed.append(normalized)
            seen.add(normalized)
    return changed, invalid, sources


def material_checks(control_plane_root: pathlib.Path) -> tuple[dict[str, bool], list[str]]:
    checks: dict[str, bool] = {}
    for stage in STAGES:
        key = f"action_{stage.replace('-', '_')}_exists"
        checks[key] = (control_plane_root / "actions" / "grimoire" / stage / "action.yml").is_file()
    checks["opencode_config_exists"] = (control_plane_root / "config" / "grimoire" / "opencode.json").is_file()
    checks["omo_config_exists"] = (control_plane_root / "config" / "grimoire" / "oh-my-openagent.jsonc").is_file()
    blockers = [name for name, ok in checks.items() if not ok]
    return checks, blockers


def output_path(raw: str, workspace: pathlib.Path) -> pathlib.Path:
    path = pathlib.Path(raw)
    if path.is_absolute():
        return path
    return workspace / path


def rel(path: pathlib.Path, root: pathlib.Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except (OSError, ValueError):
        return path.as_posix()


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            if isinstance(value, bool):
                text = "true" if value else "false"
            else:
                text = str(value)
            handle.write(f"{key}={text}\n")


def run(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    control_plane_root = pathlib.Path(args.control_plane_root).resolve()
    output = output_path(args.output, workspace)
    changed, invalid, sources = read_changed(args.changed_files, args.changed_file)
    protected_matches = [
        {"path": path, "reason": protected_reason(path)}
        for path in changed
        if protected_reason(path)
    ]
    protected_paths = [item["path"] for item in protected_matches]
    checks, material_blockers = material_checks(control_plane_root)

    if material_blockers or invalid:
        status = "blocked"
        action = "halt"
        reason_parts = []
        if material_blockers:
            reason_parts.append("trusted control-plane material incomplete: " + ", ".join(material_blockers))
        if invalid:
            reason_parts.append("invalid changed paths: " + ", ".join(invalid))
        exit_code = 1
    elif protected_paths:
        status = "protected"
        action = "halt"
        reason_parts = ["protected paths require read-only halt: " + ", ".join(protected_paths)]
        exit_code = 0
    else:
        status = "ok"
        action = "continue"
        reason_parts = ["trusted control-plane material is complete and no protected paths were touched"]
        exit_code = 0

    read_only = status != "ok"
    payload = {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "status": status,
        "action": action,
        "reason": "; ".join(reason_parts),
        "protected_paths": protected_paths,
        "protected_path_matches": protected_matches,
        "changed_files": changed,
        "changed_file_sources": sources,
        "invalid_changed_paths": invalid,
        "read_only": read_only,
        "model_execution_allowed": status == "ok",
        "write_allowed": status == "ok",
        "write_mutation_allowed": status == "ok",
        "commit_allowed": status == "ok",
        "push_allowed": status == "ok",
        "github_mutation_allowed": status == "ok",
        "comment_allowed": status == "ok",
        "push_attempts": 0,
        "base_controller_path": str(control_plane_root),
        "status_path": rel(output, workspace),
        "protected_comment_required": status == "protected",
        "protected_comment_artifact": ".omo/ci/trusted-controller-comment.md",
        "controller_checks": checks,
        "material_blockers": material_blockers,
        "protected_patterns": [
            ".github/**",
            ".opencode/**",
            "opencode.json",
            "**/AGENTS.md",
            "docs/SECURITY.md",
            "docs/REVIEW_CHECKLIST.md",
            "docs/FORKED_RELAYER_CRATE.md",
            "docs/PUBLISHING_DISABLED.md",
        ],
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    write_github_output(
        args.github_output,
        {
            "status": status,
            "action": action,
            "read_only": read_only,
            "model_execution_allowed": status == "ok",
            "write_allowed": status == "ok",
            "commit_allowed": status == "ok",
            "push_allowed": status == "ok",
            "github_mutation_allowed": status == "ok",
            "status_path": str(output),
        },
    )
    print(f"{STAGE}: status={status} action={action} output={rel(output, workspace)}")
    return exit_code


def parse_args(argv: list[str]) -> argparse.Namespace:
    default_root = pathlib.Path(__file__).resolve().parents[4]
    parser = argparse.ArgumentParser(description="Run the Grimoire trusted-controller guard.")
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--control-plane-root", default=str(default_root))
    parser.add_argument("--changed-files", action="append", default=[])
    parser.add_argument("--changed-file", action="append", default=[])
    parser.add_argument("--output", default=".omo/ci/trusted-controller-status.json")
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
