#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
from datetime import datetime, timezone
from typing import cast

STAGE = "grimoire-review"
LENSES = ["security", "correctness", "maintainability", "repo-policy"]
MARKER = "GRIMOIRE_REVIEW_DEFECT"
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


def default_control_plane_root() -> pathlib.Path:
    return pathlib.Path(__file__).resolve().parents[4]


def control_plane_root(raw: str) -> pathlib.Path:
    return pathlib.Path(raw).resolve() if raw else default_control_plane_root()


def write_json(path: pathlib.Path, payload: dict[str, object]) -> None:
    _ = payload.setdefault("schema_version", 1)
    _ = payload.setdefault("stage", STAGE)
    _ = payload.setdefault("generated_at", utc_now())
    _ = payload.setdefault("lenses", LENSES)
    _ = path.parent.mkdir(parents=True, exist_ok=True)
    _ = path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_text(path: pathlib.Path, text: str) -> None:
    _ = path.parent.mkdir(parents=True, exist_ok=True)
    _ = path.write_text(text, encoding="utf-8")


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            if isinstance(value, bool):
                text = "true" if value else "false"
            else:
                text = str(value)
            _ = handle.write(f"{key}={text}\n")


def base_payload(status: str, approval_signal: str, findings: list[dict[str, object]], mode: str) -> dict[str, object]:
    return {
        "status": status,
        "approval_signal": approval_signal,
        "read_only": True,
        "mutation_allowed": False,
        "findings": findings,
        "findings_count": len(findings),
        "fixture": mode if mode in {"clean", "defect"} else "none",
        "mode": mode,
        "review_contract": "read-only four-lens review; no edits, comments, labels, commits, pushes, or GitHub mutation",
    }


def make_finding(file_name: str, line_number: int, line_text: str) -> dict[str, object]:
    return {
        "file": file_name,
        "line": line_number,
        "severity": "medium",
        "lens": "correctness",
        "title": "Deterministic review defect marker present",
        "what": f"The local review fixture contains {MARKER}.",
        "why": "The marker is a deterministic defect fixture used to prove file and line finding emission before live team-mode wiring.",
        "suggested_fix": f"Remove {MARKER} or replace the intentionally defective fixture line with corrected code.",
        "evidence": f"{MARKER} observed at {file_name}:{line_number}: {line_text}",
    }


def scan_fixture(path: pathlib.Path) -> list[dict[str, object]]:
    text = path.read_text(encoding="utf-8", errors="replace")
    findings: list[dict[str, object]] = []
    current_file: str | None = None
    new_line: int | None = None
    saw_diff = False
    for raw_line in text.splitlines():
        if raw_line.startswith("+++ "):
            saw_diff = True
            target = raw_line[4:].strip()
            current_file = target[2:] if target.startswith("b/") else target
            continue
        if raw_line.startswith("@@ "):
            saw_diff = True
            match = re.search(r"\+(\d+)(?:,\d+)?", raw_line)
            new_line = int(match.group(1)) if match else None
            continue
        if new_line is not None:
            if raw_line.startswith("+") and not raw_line.startswith("+++"):
                if MARKER in raw_line:
                    findings.append(make_finding(current_file or str(path), new_line, raw_line[1:].strip()))
                new_line += 1
            elif raw_line.startswith("-") and not raw_line.startswith("---"):
                continue
            elif raw_line.startswith(" ") or raw_line == "":
                new_line += 1
    if findings or saw_diff:
        return findings
    for line_number, line in enumerate(text.splitlines(), start=1):
        if MARKER in line:
            findings.append(make_finding(str(path), line_number, line.strip()))
    return findings


def blocked_payload(reasons: list[str], category: str = "") -> dict[str, object]:
    payload = base_payload("blocked", "GRIMOIRE_REVIEW_BLOCKED", [], "live")
    payload["blocked_reason"] = "; ".join(reasons)
    payload["blockers"] = reasons
    if category:
        payload["blocked_reason_category"] = category
    payload["real_mode_attempted"] = True
    return payload


def load_json_value(text: str) -> object:
    return cast(object, json.loads(text))


def _review_payload_key(payload: dict[str, object]) -> str:
    return json.dumps(payload, sort_keys=True, separators=(",", ":"))


def _choose_review_payload(payloads: list[dict[str, object]]) -> dict[str, object] | None:
    if not payloads:
        return None
    unique: dict[str, dict[str, object]] = {}
    for payload in payloads:
        unique[_review_payload_key(payload)] = payload
    if len(unique) > 1:
        raise ContractError("live review output contained multiple distinct review JSON payloads")
    return payloads[-1]


def _review_payload_from_text(text: str) -> dict[str, object] | None:
    candidates = [text.strip()]
    for match in re.finditer(r"```(?:json)?\s*(\{.*?\})\s*```", text, flags=re.DOTALL | re.IGNORECASE):
        candidates.append(match.group(1))
    first = text.find("{")
    last = text.rfind("}")
    if first != -1 and last > first:
        candidates.append(text[first : last + 1])
    payloads: list[dict[str, object]] = []
    for candidate in candidates:
        if not candidate:
            continue
        try:
            loaded = load_json_value(candidate)
        except json.JSONDecodeError:
            continue
        payload = _review_payload_from_value(cast(dict[str, object], loaded))
        if payload is not None:
            payloads.append(payload)
    return _choose_review_payload(payloads)


def _review_payload_from_content(value: object) -> dict[str, object] | None:
    if isinstance(value, str):
        return _review_payload_from_text(value)
    if isinstance(value, list):
        payloads: list[dict[str, object]] = []
        for item in cast(list[object], value):
            payload = _review_payload_from_value(item)
            if payload is not None:
                payloads.append(payload)
        return _choose_review_payload(payloads)
    if isinstance(value, dict):
        return _review_payload_from_value(cast(dict[str, object], value))
    return None


def _review_payload_from_value(value: object) -> dict[str, object] | None:
    if isinstance(value, list):
        payloads: list[dict[str, object]] = []
        for item in cast(list[object], value):
            payload = _review_payload_from_value(item)
            if payload is not None:
                payloads.append(payload)
        return _choose_review_payload(payloads)
    if not isinstance(value, dict):
        return None
    payload = cast(dict[str, object], value)
    status = payload.get("status")
    if isinstance(payload.get("findings"), list) or status in {"approved", "findings"}:
        return dict(payload)
    for key in ("message", "content", "text"):
        payload_from_key = _review_payload_from_content(payload.get(key))
        if payload_from_key is not None:
            return payload_from_key
    part = payload.get("part")
    if isinstance(part, dict):
        part_payload = cast(dict[str, object], part)
        if payload.get("type") == "text" or part_payload.get("type") == "text":
            return _review_payload_from_value(part_payload)
    properties = payload.get("properties")
    if isinstance(properties, dict):
        properties_payload = cast(dict[str, object], properties)
        info = properties_payload.get("info")
        role = cast(dict[str, object], info).get("role") if isinstance(info, dict) else None
        if role == "assistant":
            for key in ("message", "content", "text", "part"):
                payload_from_key = _review_payload_from_content(properties_payload.get(key))
                if payload_from_key is not None:
                    return payload_from_key
    return None


def _review_payloads_from_jsonl(text: str) -> list[dict[str, object]]:
    payloads: list[dict[str, object]] = []
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            loaded = load_json_value(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(loaded, dict) or "type" not in loaded:
            continue
        payload = _review_payload_from_value(cast(dict[str, object], loaded))
        if payload is not None:
            payloads.append(payload)
    return payloads


def extract_json(text: str) -> dict[str, object]:
    payloads: list[dict[str, object]] = []
    direct = _review_payload_from_text(text)
    if direct is not None:
        payloads.append(direct)
    payloads.extend(_review_payloads_from_jsonl(text))
    payload = _choose_review_payload(payloads)
    if payload is None:
        raise ContractError("live review output did not contain schema-valid JSON")
    return payload


def normalize_live_payload(raw: dict[str, object]) -> dict[str, object]:
    findings_raw = raw.get("findings")
    if findings_raw is None:
        findings_raw = []
    if not isinstance(findings_raw, list):
        raise ContractError("live review JSON field findings must be an array")
    findings: list[dict[str, object]] = []
    for index, item in enumerate(cast(list[object], findings_raw)):
        if not isinstance(item, dict):
            raise ContractError(f"live review finding {index} must be an object")
        finding = dict(cast(dict[str, object], item))
        missing = [field for field in REQUIRED_FINDING_FIELDS if field not in finding]
        if missing:
            raise ContractError(f"live review finding {index} missing fields: {', '.join(missing)}")
        lens = str(finding.get("lens") or "")
        if lens not in LENSES:
            raise ContractError(f"live review finding {index} has unsupported lens: {lens}")
        findings.append(finding)
    status = str(raw.get("status") or ("findings" if findings else "approved"))
    if status not in {"approved", "findings"}:
        raise ContractError(f"live review status must be approved or findings, got {status}")
    if status == "approved" and findings:
        status = "findings"
    signal = "GRIMOIRE_REVIEW_FINDINGS_PRESENT" if findings else "GRIMOIRE_REVIEW_APPROVED"
    payload = base_payload(status, signal, findings, "live")
    payload["real_mode_attempted"] = True
    payload["live_invocation_contract"] = "opencode run, controller-owned config, read-only JSON review artifact"
    return payload


def render_live_prompt(output: pathlib.Path) -> str:
    fields = ", ".join(REQUIRED_FINDING_FIELDS)
    return f"""# Grimoire Review Stage

You are running the read-only Grimoire review stage from trusted control-plane config.
Inspect the checked-out consumer pull request workspace. Do not edit files. Do not write comments, labels, commits, pushes, or GitHub mutations.
Review through these four lenses: {', '.join(LENSES)}.

Return one JSON object only. The object schema is:
{{
  "status": "approved" or "findings",
  "findings": [
    {{"file": "relative/path", "line": 1, "severity": "low|medium|high|critical", "lens": "security|correctness|maintainability|repo-policy", "title": "short title", "what": "problem", "why": "impact", "suggested_fix": "bounded suggestion", "evidence": "specific file/line evidence"}}
  ]
}}

Every finding must include these fields: {fields}.
If no actionable findings exist, return {{"status":"approved","findings":[]}}.
The controller will write the normalized artifact to {output}.
"""


def run_live_review(workspace: pathlib.Path, output: pathlib.Path, root: pathlib.Path) -> tuple[dict[str, object], int]:
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
    blocked_category = ""
    if opencode_path is None:
        blockers.append("opencode executable is not available on PATH")
        blocked_category = "missing-runtime:opencode-unavailable"
    if blockers:
        return blocked_payload(blockers, blocked_category), 1
    assert opencode_path is not None

    prompt_path = resolve_path(".omo/ci/review-live-prompt.md", workspace)
    write_text(prompt_path, render_live_prompt(output))
    env = os.environ.copy()
    env["OPENCODE_CONFIG"] = str(opencode_config)
    _ = env.setdefault("OPENCODE_DISABLE_PROJECT_CONFIG", "1")
    _ = env.setdefault("OPENCODE_PURE", "1")
    command = [
        opencode_path,
        "run",
        "--format",
        "json",
        "--dir",
        str(workspace),
        "--model",
        "ai-relay/gpt-5.5",
        "--agent",
        "build",
        prompt_path.read_text(encoding="utf-8"),
    ]
    completed = subprocess.run(command, cwd=str(workspace), env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1800, check=False)
    if completed.returncode != 0:
        return blocked_payload(["opencode review command failed before producing a valid review artifact"], "runtime-failed:opencode-command-failed"), 1
    try:
        payload = normalize_live_payload(extract_json(completed.stdout))
    except ContractError:
        return blocked_payload(["opencode review output did not satisfy the review JSON contract"], "contract-invalid:review-json-invalid"), 1
    payload["prompt_path"] = rel(prompt_path, workspace)
    payload["controller_config"] = rel(opencode_config, root)
    payload["consumer_pr_head_config_trusted"] = False
    return payload, 0


def run(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(cast(str, args.consumer_workspace)).resolve()
    output = resolve_path(cast(str, args.output), workspace)
    fixture = cast(str, args.fixture)
    exit_code = 0
    if fixture == "clean":
        payload = base_payload("approved", "GRIMOIRE_REVIEW_APPROVED", [], "clean")
        payload["real_mode_attempted"] = False
    elif fixture == "defect":
        if not cast(str, args.fixture_input):
            payload = blocked_payload(["--fixture-input is required for defect review fixture"])
            exit_code = 1
        else:
            fixture_path = resolve_path(cast(str, args.fixture_input), workspace)
            if not fixture_path.exists():
                payload = blocked_payload([f"fixture input does not exist: {fixture_path}"])
                exit_code = 1
            else:
                findings = scan_fixture(fixture_path)
                if not findings:
                    payload = blocked_payload([f"fixture contains no {MARKER} marker: {fixture_path}"])
                    exit_code = 1
                else:
                    payload = base_payload("findings", "GRIMOIRE_REVIEW_FINDINGS_PRESENT", findings, "defect")
                    payload["real_mode_attempted"] = False
    else:
        payload, exit_code = run_live_review(workspace, output, control_plane_root(cast(str, args.control_plane_root)))

    write_json(output, payload)
    outputs = {
        "status": payload["status"],
        "approval_signal": payload["approval_signal"],
        "read_only": True,
        "mutation_allowed": False,
        "findings_count": payload["findings_count"],
        "output_path": str(output),
    }
    blocked_reason_category = payload.get("blocked_reason_category")
    if isinstance(blocked_reason_category, str) and blocked_reason_category:
        outputs["blocked_reason_category"] = blocked_reason_category
    write_github_output(cast(str | None, args.github_output), outputs)
    reason = f" reason={blocked_reason_category}" if isinstance(blocked_reason_category, str) and blocked_reason_category else ""
    print(f"{STAGE}: status={payload['status']} findings={payload['findings_count']}{reason} output={output}")
    return exit_code


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the read-only Grimoire review stage.")
    _ = parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    _ = parser.add_argument("--control-plane-root", default="")
    _ = parser.add_argument("--output", default=".omo/ci/review-findings.json")
    _ = parser.add_argument("--fixture", choices=["clean", "defect"], default="")
    _ = parser.add_argument("--fixture-input", default="")
    _ = parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
