#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import posixpath
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from typing import Any

STAGE = "grimoire-fix"


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


def default_control_plane_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[4]


def control_plane_root(raw: str) -> pathlib.Path:
    return pathlib.Path(raw).resolve() if raw else default_control_plane_root()


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


def read_list(raw_paths: list[str], workspace: pathlib.Path) -> tuple[list[str], list[str]]:
    values: list[str] = []
    invalid: list[str] = []
    seen: set[str] = set()
    for raw_path in raw_paths:
        path = resolve_path(raw_path, workspace)
        if not path.exists():
            invalid.append(raw_path)
            continue
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            normalized = normalize_path(line)
            if normalized is None:
                invalid.append(line)
            elif normalized and normalized not in seen:
                values.append(normalized)
                seen.add(normalized)
    return values, invalid


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


def cited_paths(spec_sufficiency: dict[str, object]) -> list[str]:
    paths: list[str] = []
    seen: set[str] = set()
    bindings_raw = spec_sufficiency.get("bindings")
    if not isinstance(bindings_raw, list):
        return paths
    for binding in bindings_raw:
        if not isinstance(binding, dict):
            continue
        values: list[object] = []
        if binding.get("citation"):
            values.append(binding["citation"])
        if isinstance(binding.get("citations"), list):
            values.extend(binding["citations"])  # type: ignore[arg-type]
        for value in values:
            text = str(value).split(":", 1)[0]
            normalized = normalize_path(text)
            if normalized and normalized not in seen:
                paths.append(normalized)
                seen.add(normalized)
    return paths


def direct_extra_allowed(path: str) -> bool:
    if path.startswith(("tests/", "docs/", "openspec/", "spec/", "schemas/")):
        return True
    name = pathlib.PurePosixPath(path).name
    return name.endswith(("_test.py", "_test.rs", ".test.ts", ".test.tsx", ".spec.ts", ".spec.tsx"))


def in_scope_work_exists(spec_sufficiency: dict[str, object]) -> bool:
    in_scope = spec_sufficiency.get("in_scope")
    return isinstance(in_scope, list) and bool(in_scope)


def write_json(path: pathlib.Path, payload: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: pathlib.Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def write_handoff(path: pathlib.Path, payload: dict[str, object]) -> None:
    lines = [
        "# Grimoire Fix Handoff",
        "",
        "Atlas may run `/start-work` only inside the scope guard recorded below.",
        "",
        f"Status: `{payload['status']}`",
        f"Scope OK: `{str(payload['scope_ok']).lower()}`",
        "",
        "## Allowed Paths",
    ]
    allowed_paths = payload.get("allowed_paths")
    allowed_list = allowed_paths if isinstance(allowed_paths, list) else []
    for item in allowed_list:
        lines.append(f"- `{item}`")
    if not allowed_list:
        lines.append("- No edit paths are allowed beyond a clear-noop result.")
    lines.extend(["", "## Changed Files"])
    changed_paths = payload.get("changed_files")
    changed_list = changed_paths if isinstance(changed_paths, list) else []
    for item in changed_list:
        lines.append(f"- `{item}`")
    if not changed_list:
        lines.append("- No post-fix changed files were detected; this is a clear-noop.")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            text = "true" if value is True else "false" if value is False else str(value)
            handle.write(f"{key}={text}\n")


def blocked_payload(reason: str, output: pathlib.Path, handoff: pathlib.Path, workspace: pathlib.Path) -> dict[str, object]:
    return {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "status": "blocked",
        "scope_ok": False,
        "noop": False,
        "should_commit": False,
        "should_push": False,
        "blocked_reason": reason,
        "changed_files": [],
        "allowed_paths": [],
        "violations": [],
        "invalid_direct_extras": [],
        "output_path": rel(output, workspace),
        "handoff_path": rel(handoff, workspace),
    }


def run_git(args: list[str], workspace: pathlib.Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(["git", *args], cwd=str(workspace), check=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def git_changed_paths(workspace: pathlib.Path) -> list[str]:
    result = run_git(["status", "--porcelain"], workspace)
    if result.returncode != 0:
        raise ContractError("git status failed while detecting post-fix changes")
    values: list[str] = []
    seen: set[str] = set()
    for line in result.stdout.splitlines():
        if not line:
            continue
        raw_path = line[3:].strip()
        if " -> " in raw_path:
            raw_path = raw_path.split(" -> ", 1)[1]
        normalized = normalize_path(raw_path)
        if normalized and not normalized.startswith(".omo/") and normalized not in seen:
            values.append(normalized)
            seen.add(normalized)
    return values


def render_live_prompt(spec: dict[str, object], allowed: list[str], handoff: pathlib.Path) -> str:
    return "\n".join(
        [
            "# Grimoire Fix Stage",
            "",
            "Run the Atlas-controlled fix for the in-scope Grimoire design plan.",
            "Do not commit, push, label, comment, or mutate GitHub state.",
            "Do not edit files outside the allowed path set. If no code change is required, leave the tree unchanged.",
            "",
            "## Allowed Paths",
            *(f"- `{path}`" for path in allowed),
            "",
            "## In-Scope Findings",
            json.dumps(spec.get("in_scope", []), indent=2, sort_keys=True),
            "",
            f"The controller will inspect git status after this command and write `{handoff}` plus `.omo/ci/fix-status.json`.",
        ]
    ) + "\n"


def run_live_fix(workspace: pathlib.Path, root: pathlib.Path, spec: dict[str, object], allowed: list[str], prompt_output: pathlib.Path) -> None:
    blockers: list[str] = []
    for required_env in ("AI_RELAY_API_KEY", "CF_ACCESS_CLIENT_ID", "CF_ACCESS_CLIENT_SECRET"):
        if not os.environ.get(required_env):
            blockers.append(f"{required_env} is not set")
    opencode_config = root / "config" / "grimoire" / "opencode.json"
    omo_config = root / "config" / "grimoire" / "oh-my-openagent.jsonc"
    if not opencode_config.is_file():
        blockers.append(f"controller-owned opencode config missing: {opencode_config}")
    if not omo_config.is_file():
        blockers.append(f"controller-owned OMO config missing: {omo_config}")
    opencode_path = shutil.which("opencode")
    if opencode_path is None:
        blockers.append("opencode executable is not available on the runner")
    if blockers:
        raise ContractError("; ".join(blockers))
    assert opencode_path is not None
    write_text(prompt_output, render_live_prompt(spec, allowed, prompt_output))
    env = os.environ.copy()
    env["OPENCODE_CONFIG"] = str(opencode_config)
    env.setdefault("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
    env.setdefault("OPENCODE_PURE", "1")
    completed = subprocess.run(
        [
            opencode_path,
            "run",
            "--format",
            "json",
            "--dir",
            str(workspace),
            "--model",
            "ai-relay/gpt-5.5",
            "--agent",
            "atlas",
            prompt_output.read_text(encoding="utf-8"),
        ],
        cwd=str(workspace),
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=3600,
        check=False,
    )
    if completed.returncode != 0:
        raise ContractError("opencode Atlas fix command failed before post-fix change detection")


def run(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    spec_path = resolve_path(args.spec_sufficiency, workspace)
    gap_path = resolve_path(args.spec_gap_status, workspace)
    output = resolve_path(args.output, workspace)
    handoff = resolve_path(args.handoff_output, workspace)
    live_prompt = resolve_path(".omo/ci/fix-live-prompt.md", workspace)
    try:
        spec = load_json(spec_path, "spec sufficiency artifact")
        gap = load_json(gap_path, "spec-gap status artifact") if gap_path.exists() else {"should_halt": False}
        if spec.get("spec_sufficient") is not True:
            payload = blocked_payload("spec_sufficient must be true before fix can run", output, handoff, workspace)
            write_json(output, payload)
            write_handoff(handoff, payload)
            return_code = 1
        elif gap.get("should_halt") is True:
            payload = blocked_payload("spec-gap status requested halt before fix", output, handoff, workspace)
            write_json(output, payload)
            write_handoff(handoff, payload)
            return_code = 1
        else:
            pr_touched, invalid_pr = read_list(args.pr_touched, workspace)
            direct_extra, invalid_extra_files = read_list(args.direct_extra, workspace)
            invalid_direct = sorted(path for path in direct_extra if not direct_extra_allowed(path))
            cited = cited_paths(spec)
            allowed = sorted(set(pr_touched + direct_extra + cited))
            live_fix_attempted = False
            if args.changed_files:
                changed, invalid_changed = read_list(args.changed_files, workspace)
            elif in_scope_work_exists(spec):
                live_fix_attempted = True
                run_live_fix(workspace, control_plane_root(args.control_plane_root), spec, allowed, live_prompt)
                changed = git_changed_paths(workspace)
                invalid_changed = []
            else:
                changed = []
                invalid_changed = []
            violations = sorted(path for path in changed if path not in set(allowed))
            scope_ok = not invalid_pr and not invalid_extra_files and not invalid_changed and not invalid_direct and not violations
            status = "clear-noop" if not changed and scope_ok else "fixed" if scope_ok else "scope-violation"
            payload: dict[str, object] = {
                "schema_version": 1,
                "stage": STAGE,
                "generated_at": utc_now(),
                "status": status,
                "scope_ok": scope_ok,
                "noop": status == "clear-noop",
                "clear_noop": status == "clear-noop",
                "fixed": status == "fixed",
                "should_commit": status == "fixed" and scope_ok,
                "should_push": status == "fixed" and scope_ok,
                "changed_files": changed,
                "pr_touched_paths": pr_touched,
                "direct_extra_paths": direct_extra,
                "cited_openspec_paths": cited,
                "allowed_paths": allowed,
                "violations": violations,
                "invalid_pr_touched_inputs": invalid_pr,
                "invalid_direct_extra_inputs": invalid_extra_files,
                "invalid_changed_inputs": invalid_changed,
                "invalid_direct_extras": invalid_direct,
                "live_fix_attempted": live_fix_attempted,
                "post_fix_detection": "git status --porcelain excluding .omo/** runtime artifacts" if live_fix_attempted else "changed-files input" if args.changed_files else "no in-scope work",
                "output_path": rel(output, workspace),
                "handoff_path": rel(handoff, workspace),
                "no_github_mutation_performed": True,
            }
            if live_fix_attempted:
                payload["live_prompt_path"] = rel(live_prompt, workspace)
            write_json(output, payload)
            write_handoff(handoff, payload)
            return_code = 0 if scope_ok else 1
        write_github_output(args.github_output, {"status": payload["status"], "scope_ok": payload["scope_ok"], "noop": payload["noop"], "should_commit": payload["should_commit"], "should_push": payload["should_push"], "output_path": str(output), "handoff_path": str(handoff)})
        print(f"{STAGE}: status={payload['status']} scope_ok={str(payload['scope_ok']).lower()} output={output}")
        return return_code
    except ContractError as exc:
        payload = blocked_payload(str(exc), output, handoff, workspace)
        write_json(output, payload)
        write_handoff(handoff, payload)
        write_github_output(args.github_output, {"status": "blocked", "scope_ok": False, "noop": False, "should_commit": False, "should_push": False, "output_path": str(output), "handoff_path": str(handoff)})
        print(f"{STAGE}: blocked: {exc}", file=sys.stderr)
        return 1


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the Grimoire fix scope/no-op guard.")
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--control-plane-root", default="")
    parser.add_argument("--spec-sufficiency", default=".omo/ci/spec-sufficiency.json")
    parser.add_argument("--spec-gap-status", default=".omo/ci/spec-gap-status.json")
    parser.add_argument("--output", default=".omo/ci/fix-status.json")
    parser.add_argument("--handoff-output", default=".omo/ci/fix-handoff-prompt.md")
    parser.add_argument("--pr-touched", action="append", default=[])
    parser.add_argument("--direct-extra", action="append", default=[])
    parser.add_argument("--changed-files", action="append", default=[])
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
