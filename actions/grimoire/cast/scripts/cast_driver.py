#!/usr/bin/env python3
# pyright: reportAny=false, reportExplicitAny=false, reportUnknownArgumentType=false, reportUnknownMemberType=false, reportUnknownVariableType=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import base64
import json
import os
import pathlib
import re
import subprocess
import sys
import urllib.error
import urllib.request
from datetime import datetime, timezone
from typing import Any

STAGE = "grimoire-cast"
VERIFY_STAGE = "grimoire-verify"
TRUSTED_STAGE = "grimoire-trusted-controller"
STAGE_ORDER = [
    "labels:running",
    "review",
    "design",
    "out-of-scope-issues",
    "spec-gap-or-fix",
    "boulder",
    "verify",
    "terminate-or-loop",
]
APPROVE_LENSES = ("f1_oracle", "f2_quality", "f3_real_qa", "f4_scope")
OUT_OF_SCOPE_MARKER_PREFIX = "grimoire-out-of-scope"
SPEC_GAP_COMMENT_MARKER = "<!-- grimoire-spec-gap -->"
TOKEN_PATTERNS = (
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"(?i)bearer\s+[A-Za-z0-9._~-]{16,}"),
    re.compile(r"(?i)(token|secret|password|api[_-]?key)\s*[:=]\s*\S+"),
)
FORBIDDEN_INPUTS = ("mode", "dry_run", "dry-run", "allow_live", "allow-live", "simulate", "simulation")
FORBIDDEN_EVENTS = ("pull_request_target", "workflow_dispatch", "push")
APP_TOKEN_STEP_ID = "grimoire-app-token"
APP_TOKEN_EXPR = "${{ steps.grimoire-app-token.outputs.token }}"
APP_TOKEN_ACTION_RE = re.compile(r"actions/create-github-app-token@([0-9A-Fa-f]{40})")
FORBIDDEN_PRIVILEGED_AUTH_MARKERS = (
    "GRIMOIRE_PAT",
    "CODEX_LOOP_PAT",
    "GITHUB_TOKEN",
    "github.token",
    "steps.auth.outputs.github_pat",
)
GITHUB_API = "https://api.github.com"


class ContractError(Exception):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def bool_text(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    text = str(value).strip().lower()
    if text in {"true", "1", "yes"}:
        return "true"
    if text in {"false", "0", "no", ""}:
        return "false"
    return str(value)


def as_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    return str(value).strip().lower() in {"true", "1", "yes"}


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


def read_json(path: pathlib.Path, label: str) -> dict[str, Any]:
    if not path.exists():
        raise ContractError(f"{label} does not exist: {path}")
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ContractError(f"{label} is not valid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise ContractError(f"{label} must be a JSON object")
    return payload


def write_json(path: pathlib.Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_outputs(path: str | None, values: dict[str, Any]) -> None:
    if not path:
        return
    output_path = pathlib.Path(path)
    with output_path.open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={bool_text(value)}\n")


def base_payload(status: str) -> dict[str, Any]:
    return {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "status": status,
        "stage_order": STAGE_ORDER,
    }


def validate_trusted_payload(payload: dict[str, Any]) -> None:
    if payload.get("schema_version") != 1 or payload.get("stage") != TRUSTED_STAGE:
        raise ContractError("trusted-controller status has the wrong schema or stage")
    if payload.get("status") != "ok" or payload.get("action") != "continue":
        raise ContractError(f"trusted-controller refused cast: status={payload.get('status')} action={payload.get('action')}")
    required = ("model_execution_allowed", "write_allowed", "commit_allowed", "push_allowed", "github_mutation_allowed")
    missing = [field for field in required if payload.get(field) is not True]
    if missing:
        raise ContractError("trusted-controller capabilities are not sufficient: " + ", ".join(missing))


def run_preflight(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    output = resolve_path(args.output, workspace)
    try:
        if args.trusted_outcome and args.trusted_outcome != "success":
            raise ContractError(f"trusted-controller step outcome was {args.trusted_outcome}")
        if not args.trusted_status_path:
            raise ContractError("trusted-controller status path output is missing")
        trusted_path = resolve_path(args.trusted_status_path, workspace)
        trusted = read_json(trusted_path, "trusted-controller status")
        validate_trusted_payload(trusted)
        expected_pairs = {
            "status": args.trusted_status,
            "action": args.trusted_action,
            "model_execution_allowed": args.model_execution_allowed,
            "write_allowed": args.write_allowed,
            "commit_allowed": args.commit_allowed,
            "push_allowed": args.push_allowed,
            "github_mutation_allowed": args.github_mutation_allowed,
        }
        missing_outputs = [name for name, value in expected_pairs.items() if str(value).strip() == ""]
        if missing_outputs:
            raise ContractError("trusted-controller required outputs are missing: " + ", ".join(missing_outputs))
        mismatched = []
        for name, value in expected_pairs.items():
            trusted_value = trusted.get(name)
            if isinstance(trusted_value, bool):
                if as_bool(value) is not trusted_value:
                    mismatched.append(name)
            elif str(trusted_value) != str(value):
                mismatched.append(name)
        if mismatched:
            raise ContractError("trusted-controller outputs do not match status artifact: " + ", ".join(mismatched))
        payload = base_payload("ok")
        payload.update(
            {
                "can_continue": True,
                "trusted_status_path": rel(trusted_path, workspace),
                "trusted_controller_status": trusted.get("status"),
                "does_not_rerun_trusted_controller": True,
            }
        )
    except ContractError as exc:
        payload = base_payload("blocked")
        payload.update(
            {
                "can_continue": False,
                "blocked_reason": str(exc),
                "does_not_rerun_trusted_controller": True,
            }
        )
    write_json(output, payload)
    write_outputs(args.github_output, {"status": payload["status"], "can_continue": payload["can_continue"], "output_path": str(output)})
    print(f"{STAGE}: preflight status={payload['status']} output={output}")
    return 0


def sanitize(value: Any, max_len: int = 500) -> str:
    text = str(value or "").replace("\r", " ").strip()
    text = re.sub(r"\s+", " ", text)
    for pattern in TOKEN_PATTERNS:
        text = pattern.sub("[REDACTED]", text)
    if len(text) > max_len:
        return text[: max_len - 1].rstrip() + "…"
    return text


def marker_for(fingerprint: str) -> str:
    return f"<!-- {OUT_OF_SCOPE_MARKER_PREFIX}:{fingerprint} -->"


def issue_title(item: dict[str, Any]) -> str:
    raw = sanitize(item.get("title") or "Out-of-scope Grimoire finding", 96)
    raw = raw.removeprefix("[out-of-scope]").removeprefix("[Out-of-scope]").strip(" :-") or "Out-of-scope Grimoire finding"
    return f"[Grimoire out-of-scope] {raw}"


def issue_body(item: dict[str, Any], repository: str, pr_number: str) -> str:
    fingerprint = sanitize(item.get("fingerprint"), 80)
    finding = item.get("finding") if isinstance(item.get("finding"), dict) else {}
    what = sanitize(finding.get("what") if isinstance(finding, dict) else item.get("what"), 800)
    why = sanitize(finding.get("why") if isinstance(finding, dict) else item.get("why"), 800)
    path = sanitize(item.get("path") or item.get("location") or "unknown", 240)
    owner = sanitize(item.get("suggested_owner") or "repo-maintainer", 120)
    return "\n".join(
        [
            marker_for(fingerprint),
            "",
            "Grimoire classified this finding as outside the active OpenSpec/OMO scope after the design stage.",
            "",
            f"- Repository: `{sanitize(repository, 160)}`",
            f"- Originating PR: `#{sanitize(pr_number, 40)}`",
            f"- Affected path: `{path}`",
            f"- Suggested owner or label: `{owner}`",
            "",
            "Problem:",
            what or "Out-of-scope issue details were unavailable after redaction.",
            "",
            "Why it matters:",
            why or "The finding should be tracked separately instead of fixed inside the current Grimoire loop.",
            "",
            "This Issue is intentionally short and redacted. It contains no raw logs, token values, token prefixes, token lengths, token hashes, or PR-head file writes.",
        ]
    ) + "\n"


def load_issue_ledger(path: pathlib.Path) -> dict[str, Any]:
    if not path.exists():
        return {"schema_version": 1, "fingerprints": {}}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError:
        return {"schema_version": 1, "fingerprints": {}}
    if not isinstance(payload, dict):
        return {"schema_version": 1, "fingerprints": {}}
    if not isinstance(payload.get("fingerprints"), dict):
        payload["fingerprints"] = {}
    return payload


def github_request(method: str, path: str, token: str, payload: dict[str, Any] | None = None) -> Any:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        GITHUB_API + path,
        data=data,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
            "Content-Type": "application/json",
            "User-Agent": "grimoire-cast-driver",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read().decode("utf-8")
        return json.loads(body) if body else {}


def is_remote_repository(repository: str) -> bool:
    return bool(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository or ""))


def find_remote_issue(repository: str, fingerprint: str, token: str) -> str:
    marker = marker_for(fingerprint)
    for state in ("open", "closed"):
        for page in range(1, 6):
            issues = github_request("GET", f"/repos/{repository}/issues?state={state}&per_page=100&page={page}", token)
            if not isinstance(issues, list) or not issues:
                break
            for issue in issues:
                if not isinstance(issue, dict) or "pull_request" in issue:
                    continue
                if marker in str(issue.get("body") or ""):
                    return str(issue.get("html_url") or issue.get("url") or "")
    return ""


def validate_spec_gap_comment_state(decision: dict[str, Any], gap: dict[str, Any], body: str) -> None:
    if decision.get("schema_version") != 1 or decision.get("stage") != STAGE:
        raise ContractError("cast decision has the wrong schema or stage")
    if decision.get("decision") != "spec-gap-halt" or decision.get("conclusion") != "neutral" or decision.get("exit_code") != 0:
        raise ContractError("cast decision is not a neutral spec-gap advisory")
    if decision.get("label_transition") != "spec-needed":
        raise ContractError("cast decision did not request the spec-needed advisory transition")
    if gap.get("schema_version") != 1 or gap.get("stage") != "grimoire-spec-gap":
        raise ContractError("spec-gap status has the wrong schema or stage")
    if gap.get("status") != "halt" or gap.get("should_halt") is not True:
        raise ContractError("spec-gap status is not a halt advisory")
    if gap.get("advisory") is not True or gap.get("should_comment") is not True:
        raise ContractError("spec-gap status did not request an advisory comment")
    if gap.get("no_code_or_push_action") is not True:
        raise ContractError("spec-gap status must continue forbidding code or push action")
    if not body.splitlines() or body.splitlines()[0] != SPEC_GAP_COMMENT_MARKER:
        raise ContractError("spec-gap comment marker must be the first line")
    if body.count(SPEC_GAP_COMMENT_MARKER) != 1:
        raise ContractError("spec-gap comment must contain exactly one marker")


def read_spec_gap_comment(path: pathlib.Path) -> str:
    if not path.is_file():
        raise ContractError(f"spec-gap comment file does not exist: {path}")
    return path.read_text(encoding="utf-8")


def find_spec_gap_comment(repository: str, pr_number: str, token: str) -> tuple[dict[str, Any] | None, int]:
    pages_read = 0
    for page in range(1, 101):
        comments = github_request("GET", f"/repos/{repository}/issues/{pr_number}/comments?per_page=100&page={page}", token)
        pages_read = page
        if not isinstance(comments, list):
            raise ContractError("GitHub issue comments API did not return a list")
        if not comments:
            break
        for comment in comments:
            if not isinstance(comment, dict):
                continue
            if SPEC_GAP_COMMENT_MARKER in str(comment.get("body") or ""):
                if comment.get("id") is None:
                    raise ContractError("existing spec-gap comment is missing an id")
                return comment, pages_read
        if len(comments) < 100:
            break
    return None, pages_read


def upsert_spec_gap_comment(repository: str, pr_number: str, token: str, body: str) -> dict[str, Any]:
    existing, pages_read = find_spec_gap_comment(repository, pr_number, token)
    if existing is not None:
        comment_id = str(existing["id"])
        response = github_request("PATCH", f"/repos/{repository}/issues/comments/{comment_id}", token, {"body": body})
        operation = "patched"
    else:
        response = github_request("POST", f"/repos/{repository}/issues/{pr_number}/comments", token, {"body": body})
        operation = "created"
    if not isinstance(response, dict):
        raise ContractError("GitHub issue comments API did not return a comment object")
    return {
        "operation": operation,
        "comment_id": str(response.get("id") or (existing or {}).get("id") or ""),
        "comment_url": str(response.get("html_url") or response.get("url") or ""),
        "pages_read": pages_read,
    }


def run_upsert_spec_gap_comment(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    output = resolve_path(args.output, workspace)
    payload = base_payload("blocked")
    exit_code = 1
    try:
        if not as_bool(args.github_mutation_allowed):
            payload = base_payload("skipped")
            payload.update(
                {
                    "comment_mutation_attempted": False,
                    "operation": "skipped",
                    "reason": "github mutation not allowed",
                    "repository": args.repository,
                    "pr_number": str(args.pr_number),
                }
            )
            exit_code = 0
        else:
            if not is_remote_repository(args.repository) or str(args.pr_number) == "0":
                raise ContractError("spec-gap comment upsert requires a remote repository and pull request number")
            token = os.environ.get("GRIMOIRE_GITHUB_PAT", "")
            if not token:
                raise ContractError("spec-gap comment upsert requires resolved Grimoire App installation token auth")
            decision = read_json(resolve_path(args.decision, workspace), "cast decision")
            gap = read_json(resolve_path(args.spec_gap_status, workspace), "spec-gap status")
            raw_comment_path = args.comment_path or str(gap.get("comment_path") or "")
            if not raw_comment_path:
                raise ContractError("spec-gap comment path is missing")
            comment_path = resolve_path(raw_comment_path, workspace)
            body = read_spec_gap_comment(comment_path)
            validate_spec_gap_comment_state(decision, gap, body)
            result = upsert_spec_gap_comment(args.repository, str(args.pr_number), token, body)
            payload = base_payload("ok")
            payload.update(
                {
                    "comment_mutation_attempted": True,
                    "operation": result["operation"],
                    "repository": args.repository,
                    "pr_number": str(args.pr_number),
                    "comment_marker": SPEC_GAP_COMMENT_MARKER,
                    "comment_path": rel(comment_path, workspace),
                    "comment_id": result["comment_id"],
                    "comment_url": result["comment_url"],
                    "pages_read": result["pages_read"],
                }
            )
            exit_code = 0
    except (ContractError, urllib.error.URLError, urllib.error.HTTPError) as exc:
        payload = base_payload("blocked")
        payload.update(
            {
                "comment_mutation_attempted": False,
                "operation": "blocked",
                "repository": args.repository,
                "pr_number": str(args.pr_number),
                "blocked_reason": str(exc),
            }
        )
        exit_code = 1
    write_json(output, payload)
    write_outputs(args.github_output, {"status": payload["status"], "operation": payload.get("operation", ""), "comment_mutation_attempted": payload.get("comment_mutation_attempted", False), "output_path": str(output)})
    print(f"{STAGE}: spec-gap-comment status={payload['status']} operation={payload.get('operation', '')} output={output}")
    return exit_code


def run_file_issues(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    design_path = resolve_path(args.design_path, workspace)
    output = resolve_path(args.output, workspace)
    ledger_path = resolve_path(args.ledger, workspace)
    try:
        design = read_json(design_path, "design output")
        if design.get("stage") != "grimoire-design":
            raise ContractError("design output has the wrong stage")
        raw_items = design.get("out_of_scope")
        if not isinstance(raw_items, list):
            raise ContractError("design output out_of_scope must be an array")
        ledger = load_issue_ledger(ledger_path)
        fingerprints = ledger["fingerprints"]
        token = os.environ.get("GRIMOIRE_GITHUB_PAT", "")
        remote_enabled = bool(token and is_remote_repository(args.repository) and str(args.pr_number) != "0")
        if is_remote_repository(args.repository) and str(args.pr_number) != "0" and not token:
            raise ContractError("out-of-scope Issue filing requires resolved Grimoire App installation token auth")
        records: list[dict[str, Any]] = []
        created = 0
        deduped = 0
        for raw in raw_items:
            if not isinstance(raw, dict):
                raise ContractError("out_of_scope entries must be objects")
            fingerprint = sanitize(raw.get("fingerprint"), 80)
            if not fingerprint:
                raise ContractError("out_of_scope entry missing stable fingerprint")
            title = issue_title(raw)
            body = issue_body(raw, args.repository, str(args.pr_number))
            if fingerprint in fingerprints:
                deduped += 1
                records.append({"fingerprint": fingerprint, "status": "deduped-local", "issue_url": fingerprints[fingerprint].get("issue_url", ""), "title": title})
                continue
            remote_issue = ""
            if remote_enabled:
                remote_issue = find_remote_issue(args.repository, fingerprint, token)
            if remote_issue:
                fingerprints[fingerprint] = {"issue_url": remote_issue, "deduped_at": utc_now()}
                deduped += 1
                records.append({"fingerprint": fingerprint, "status": "deduped-remote", "issue_url": remote_issue, "title": title})
                continue
            issue_url = ""
            if remote_enabled:
                response = github_request("POST", f"/repos/{args.repository}/issues", token, {"title": title, "body": body})
                if not isinstance(response, dict) or not response.get("html_url"):
                    raise ContractError("GitHub Issues API did not return an issue URL")
                issue_url = str(response["html_url"])
                status = "created"
                created += 1
            else:
                status = "recorded-local-intent"
            fingerprints[fingerprint] = {"issue_url": issue_url, "recorded_at": utc_now(), "title": title}
            records.append({"fingerprint": fingerprint, "status": status, "issue_url": issue_url, "title": title, "marker": marker_for(fingerprint)})
        ledger["updated_at"] = utc_now()
        write_json(ledger_path, ledger)
        payload = base_payload("ok")
        payload.update(
            {
                "issue_write_only": True,
                "pr_head_write_attempted": False,
                "remote_issue_mutation_attempted": remote_enabled and bool(raw_items),
                "design_stage_complete": True,
                "repository": args.repository,
                "pr_number": str(args.pr_number),
                "issue_count": len(records),
                "created_count": created,
                "deduped_count": deduped,
                "records": records,
                "ledger_path": rel(ledger_path, workspace),
            }
        )
        exit_code = 0
    except (ContractError, urllib.error.URLError, urllib.error.HTTPError) as exc:
        payload = base_payload("blocked")
        payload.update(
            {
                "issue_write_only": True,
                "pr_head_write_attempted": False,
                "blocked_reason": str(exc),
                "issue_count": 0,
                "created_count": 0,
                "deduped_count": 0,
                "records": [],
            }
        )
        exit_code = 1
    write_json(output, payload)
    write_outputs(args.github_output, {"status": payload["status"], "issue_count": payload["issue_count"], "created_count": payload["created_count"], "deduped_count": payload["deduped_count"], "output_path": str(output)})
    print(f"{STAGE}: out-of-scope-issues status={payload['status']} count={payload['issue_count']} output={output}")
    return exit_code


def run_boulder(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    fix_path = resolve_path(args.fix_status, workspace)
    output = resolve_path(args.output, workspace)
    try:
        fix = read_json(fix_path, "fix status")
        if fix.get("stage") != "grimoire-fix":
            raise ContractError("fix status has the wrong stage")
        status = str(fix.get("status"))
        if status == "clear-noop":
            boulder_status = "skipped-clear-noop"
            required = False
        elif status == "fixed" and fix.get("scope_ok") is True:
            boulder_status = "completed"
            required = True
        else:
            raise ContractError(f"boulder cannot run after fix status {status}")
        payload = base_payload(boulder_status)
        payload.update(
            {
                "boulder_required": required,
                "continuation_state": "completed" if required else "not-required",
                "wall_clock_liveness_guard_seconds": args.timeout_minutes * 60,
                "semantic_iteration_cap": False,
                "source_fix_status": rel(fix_path, workspace),
            }
        )
        exit_code = 0
    except ContractError as exc:
        payload = base_payload("blocked")
        payload.update({"boulder_required": False, "blocked_reason": str(exc), "semantic_iteration_cap": False})
        exit_code = 1
    write_json(output, payload)
    write_outputs(args.github_output, {"status": payload["status"], "boulder_required": payload.get("boulder_required", False), "output_path": str(output)})
    print(f"{STAGE}: boulder status={payload['status']} output={output}")
    return exit_code


def verdict_approved(payload: dict[str, Any]) -> bool:
    if payload.get("schema_version") != 1 or payload.get("stage") != VERIFY_STAGE:
        return False
    notes = payload.get("notes")
    if not isinstance(notes, dict):
        return False
    for lens in APPROVE_LENSES:
        if payload.get(lens) not in {"APPROVE", "REJECT"}:
            return False
        if not isinstance(notes.get(lens), dict):
            return False
    return all(payload.get(lens) == "APPROVE" for lens in APPROVE_LENSES) and payload.get("approved") is True


def design_has_no_actionable_work(payload: dict[str, Any]) -> bool:
    empty_work_fields = ("in_scope", "bindings", "target_paths", "allowed_write_paths")
    return (
        payload.get("schema_version") == 1
        and payload.get("stage") == "grimoire-design"
        and payload.get("status") == "sufficient"
        and payload.get("spec_sufficient") is True
        and payload.get("should_halt") is False
        and all(isinstance(payload.get(field), list) and not payload.get(field) for field in empty_work_fields)
    )


def stage_blockers(args: argparse.Namespace, workspace: pathlib.Path) -> list[str]:
    blockers: list[str] = []
    preflight = read_json(resolve_path(args.preflight_status, workspace), "cast preflight")
    if preflight.get("status") != "ok" or preflight.get("can_continue") is not True:
        blockers.append("trusted-controller preflight did not allow cast")
        return blockers
    review = read_json(resolve_path(args.review_status, workspace), "review output")
    if args.review_outcome != "success" or review.get("status") not in {"approved", "findings"}:
        blockers.append(f"review failed closed: outcome={args.review_outcome} status={review.get('status')}")
    design = read_json(resolve_path(args.design_status, workspace), "design output")
    if design.get("status") == "blocked":
        blockers.append("design failed closed")
    issues = read_json(resolve_path(args.issue_status, workspace), "out-of-scope issue status")
    if issues.get("status") != "ok":
        blockers.append("out-of-scope Issue filing did not complete")
    return blockers


def run_decide(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    output = resolve_path(args.output, workspace)
    payload = base_payload("fizzled")
    exit_code = 1
    label_transition = "fizzled"
    decision = "fizzled"
    terminal = False
    should_push = False
    conclusion = "failure"
    reasons: list[str] = []
    try:
        blockers = stage_blockers(args, workspace)
        if blockers:
            reasons.extend(blockers)
        design = read_json(resolve_path(args.design_status, workspace), "design output") if not blockers or pathlib.Path(resolve_path(args.design_status, workspace)).exists() else {}
        if not reasons and (design.get("should_halt") is True or design.get("spec_sufficient") is not True):
            gap = read_json(resolve_path(args.spec_gap_status, workspace), "spec-gap status")
            if gap.get("should_halt") is True and gap.get("status") == "halt":
                decision = "spec-gap-halt"
                conclusion = "neutral"
                label_transition = "spec-needed"
                exit_code = 0
                reasons.append("design requested spec-gap halt before code changes")
            else:
                reasons.append("design requested halt but spec-gap artifact was not a halt")
        if not reasons and design_has_no_actionable_work(design):
            decision = "no-actionable-work"
            conclusion = "neutral"
            label_transition = "spec-needed"
            exit_code = 0
            reasons.append("design found no actionable in-scope work with sufficient spec")
        if not reasons:
            fix = read_json(resolve_path(args.fix_status, workspace), "fix status")
            if args.fix_outcome != "success" or fix.get("scope_ok") is not True or fix.get("status") not in {"clear-noop", "fixed"}:
                reasons.append(f"fix failed closed: outcome={args.fix_outcome} status={fix.get('status')}")
            else:
                boulder = read_json(resolve_path(args.boulder_status, workspace), "boulder status")
                if fix.get("status") == "fixed" and boulder.get("status") != "completed":
                    reasons.append("boulder did not complete before verify")
                else:
                    verdict = read_json(resolve_path(args.verdict_status, workspace), "verify verdict")
                    if args.verify_outcome != "success" or not verdict_approved(verdict):
                        reasons.append(f"verify rejected or malformed verdict: outcome={args.verify_outcome}")
                    elif fix.get("status") == "clear-noop":
                        decision = "clear-noop-terminal"
                        conclusion = "success"
                        label_transition = "done"
                        terminal = True
                        exit_code = 0
                    elif fix.get("status") == "fixed":
                        decision = "scoped-push"
                        conclusion = "success"
                        label_transition = "keep-running"
                        should_push = True
                        exit_code = 0
        if reasons and decision == "fizzled":
            label_transition = "fizzled"
        payload.update(
            {
                "status": "ok" if exit_code == 0 else "fizzled",
                "decision": decision,
                "conclusion": conclusion,
                "terminal": terminal,
                "should_push": should_push,
                "label_transition": label_transition,
                "exit_code": exit_code,
                "reasons": reasons,
                "clear_noop_terminal_semantics": "clear-noop + all APPROVE -> Cast label, no commit, no push",
                "fixed_push_semantics": "fixed + all APPROVE -> exactly one scoped bot commit and push, then pull_request.synchronize re-review",
                "reject_or_halt_semantics": "REJECT, malformed, missing, protected, or non-spec-gap halt states fail closed to Fizzled",
            }
        )
    except ContractError as exc:
        payload.update({"decision": "fizzled", "conclusion": "failure", "terminal": False, "should_push": False, "label_transition": "fizzled", "exit_code": 1, "reasons": [str(exc)]})
    write_json(output, payload)
    write_outputs(args.github_output, {"status": payload["status"], "decision": payload["decision"], "terminal": payload["terminal"], "should_push": payload["should_push"], "label_transition": payload["label_transition"], "exit_code": payload["exit_code"], "output_path": str(output)})
    print(f"{STAGE}: decision={payload['decision']} status={payload['status']} output={output}")
    return 0


def run_git(args: list[str], cwd: pathlib.Path, token: str | None = None) -> subprocess.CompletedProcess[str]:
    command = ["git"]
    if token:
        auth = base64.b64encode(f"x-access-token:{token}".encode("utf-8")).decode("ascii")
        command.extend(["-c", f"http.https://github.com/.extraheader=AUTHORIZATION: basic {auth}"])
    command.extend(args)
    return subprocess.run(command, cwd=str(cwd), check=False, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def runtime_artifact_path(path: str) -> bool:
    normalized = path.replace("\\", "/").rstrip("/")
    return normalized == ".omo" or normalized.startswith(".omo/")


def changed_paths(workspace: pathlib.Path) -> list[str]:
    result = run_git(["status", "--porcelain"], workspace)
    if result.returncode != 0:
        raise ContractError("git status failed while checking scoped push")
    paths: list[str] = []
    for line in result.stdout.splitlines():
        if not line:
            continue
        path = line[3:].strip()
        if " -> " in path:
            path = path.split(" -> ", 1)[1]
        normalized = path.replace("\\", "/").rstrip("/")
        if normalized and not runtime_artifact_path(normalized):
            paths.append(normalized)
    return paths


def run_push(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    decision_path = resolve_path(args.decision, workspace)
    output = resolve_path(args.output, workspace)
    payload = base_payload("blocked")
    exit_code = 1
    try:
        decision = read_json(decision_path, "cast decision")
        if decision.get("decision") != "scoped-push" or decision.get("should_push") is not True:
            payload.update({"status": "skipped", "push_attempted": False, "reason": "decision does not request scoped push"})
            exit_code = 0
        else:
            token = os.environ.get("GRIMOIRE_GITHUB_PAT", "")
            if not token:
                raise ContractError("scoped push requires resolved Grimoire App installation token auth")
            paths = changed_paths(workspace)
            if not paths:
                raise ContractError("scoped push refused empty commit")
            expected_head = args.head_sha.strip()
            if expected_head:
                head_result = run_git(["rev-parse", "HEAD"], workspace)
                if head_result.returncode != 0 or head_result.stdout.strip() != expected_head:
                    raise ContractError("consumer checkout HEAD does not match requested head_sha")
            _ = run_git(["config", "user.name", "grimoire-bot"], workspace)
            _ = run_git(["config", "user.email", "grimoire-bot@dongwontuna-labs.invalid"], workspace)
            add_result = run_git(["add", "--", *paths], workspace)
            if add_result.returncode != 0:
                raise ContractError("git add failed for scoped push paths")
            commit_result = run_git(["commit", "-m", "chore(grimoire): apply scoped cast fix"], workspace)
            if commit_result.returncode != 0:
                raise ContractError("git commit failed for scoped push")
            commit_sha = run_git(["rev-parse", "HEAD"], workspace).stdout.strip()
            ref = args.consumer_ref.strip()
            if not ref:
                raise ContractError("consumer_ref is required for scoped push")
            push_result = run_git(["push", "origin", f"HEAD:{ref}"], workspace, token=token)
            if push_result.returncode != 0:
                raise ContractError("git push failed for scoped bot commit")
            payload.update(
                {
                    "status": "pushed",
                    "push_attempted": True,
                    "push_count": 1,
                    "empty_commit": False,
                    "changed_files": paths,
                    "commit_sha": commit_sha,
                    "next_expected_event": "pull_request.synchronize",
                }
            )
            exit_code = 0
    except ContractError as exc:
        payload.update({"status": "blocked", "push_attempted": False, "blocked_reason": str(exc), "push_count": 0, "empty_commit": False})
    write_json(output, payload)
    write_outputs(args.github_output, {"status": payload["status"], "push_attempted": payload.get("push_attempted", False), "push_count": payload.get("push_count", 0), "output_path": str(output)})
    print(f"{STAGE}: push status={payload['status']} output={output}")
    return exit_code


def legacy_exit_code_is_zero(value: Any) -> bool:
    if isinstance(value, bool):
        return False
    if isinstance(value, int):
        return value == 0
    if isinstance(value, str):
        return value.strip() == "0"
    return False


def complete_decision_conclusion(decision: dict[str, Any]) -> tuple[str, list[str]]:
    if "conclusion" not in decision:
        if legacy_exit_code_is_zero(decision.get("exit_code")):
            return "success", []
        return "failure", ["decision conclusion is missing and legacy exit_code is not zero"]
    conclusion = decision.get("conclusion")
    if not isinstance(conclusion, str):
        return "failure", ["decision conclusion must be a string"]
    normalized = conclusion.strip().lower()
    if normalized in {"success", "neutral", "failure"}:
        return normalized, []
    return "failure", [f"decision conclusion is unsupported: {sanitize(normalized, 80)}"]


def first_reason(reasons: Any) -> str:
    if isinstance(reasons, list):
        for reason in reasons:
            text = sanitize(reason, 160)
            if text:
                return text
    return ""


def run_complete(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    output = resolve_path(args.output, workspace)
    final = base_payload("fizzled")
    final.update({"decision": None, "terminal": False, "should_push": False, "conclusion": "failure"})
    exit_code = 1
    try:
        decision = read_json(resolve_path(args.decision, workspace), "cast decision")
        final.update({"decision": decision.get("decision"), "terminal": decision.get("terminal"), "should_push": decision.get("should_push")})
        conclusion, conclusion_reasons = complete_decision_conclusion(decision)
        final["conclusion"] = conclusion
        reasons = conclusion_reasons or decision.get("reasons", [])
        if conclusion_reasons:
            final["status"] = "fizzled"
            final["conclusion"] = "failure"
            final["reasons"] = conclusion_reasons
            final["summary"] = f"cast failed closed: {first_reason(conclusion_reasons)}"
            exit_code = 1
        elif conclusion == "failure":
            final["status"] = "fizzled"
            final["reasons"] = reasons if isinstance(reasons, list) else []
            reason = first_reason(final["reasons"])
            final["summary"] = f"cast failed closed: {reason}" if reason else "cast failed closed"
            exit_code = 1
        elif decision.get("decision") == "scoped-push":
            if conclusion != "success":
                final["status"] = "fizzled"
                final["conclusion"] = "failure"
                final["reasons"] = ["scoped-push decision conclusion must be success"]
                final["summary"] = "cast failed closed: scoped-push decision conclusion must be success"
                exit_code = 1
            else:
                push = read_json(resolve_path(args.push_status, workspace), "push status")
                final["push_status"] = push.get("status")
                final["push_count"] = push.get("push_count", 0)
                if push.get("status") != "pushed" or push.get("push_count") != 1:
                    final["status"] = "fizzled"
                    final["conclusion"] = "failure"
                    final["reasons"] = ["scoped push did not complete exactly once"]
                    final["summary"] = "scoped push did not complete exactly once"
                    exit_code = 1
                else:
                    final["status"] = "awaiting-synchronize"
                    final["next_expected_event"] = "pull_request.synchronize"
                    final["summary"] = "scoped push completed once; awaiting pull_request.synchronize"
                    exit_code = 0
        elif final.get("should_push") is True:
            final["status"] = "fizzled"
            final["conclusion"] = "failure"
            final["reasons"] = ["non-scoped decision requested push"]
            final["summary"] = "cast failed closed: non-scoped decision requested push"
            exit_code = 1
        elif conclusion == "neutral":
            final["status"] = "advisory"
            reason = first_reason(reasons)
            final["summary"] = f"cast completed with advisory decision: {reason}" if reason else "cast completed with advisory decision"
            exit_code = 0
        else:
            final["status"] = "terminal"
            final["summary"] = "cast completed successfully without a push"
            exit_code = 0
    except ContractError as exc:
        final["status"] = "fizzled"
        final["conclusion"] = "failure"
        final["reasons"] = [str(exc)]
        final["summary"] = f"cast failed closed: {sanitize(exc, 160)}"
        exit_code = 1
    write_json(output, final)
    write_outputs(
        args.github_output,
        {
            "status": final["status"],
            "decision": final["decision"],
            "terminal": final.get("terminal", False),
            "conclusion": final["conclusion"],
            "summary": final["summary"],
            "output_path": str(output),
        },
    )
    print(f"{STAGE}: final status={final['status']} decision={final['decision']} conclusion={final['conclusion']} output={output}")
    return exit_code


def workflow_input_block(text: str) -> str:
    match = re.search(r"(?ms)^\s*workflow_call\s*:\s*(.*?)(?:^\s{2}[A-Za-z0-9_-]+\s*:|\Z)", text)
    if not match:
        raise ContractError("workflow must declare on.workflow_call")
    return match.group(1)


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def workflow_step_block(text: str, name: str) -> str:
    lines = text.splitlines()
    name_pattern = re.compile(rf"^\s+(?:-\s*)?name\s*:\s*{re.escape(name)}\s*$")
    for index, line in enumerate(lines):
        if not name_pattern.match(line):
            continue
        start = index
        while start > 0 and not re.match(r"^\s*-\s+", lines[start]):
            start -= 1
        base_indent = indent_of(lines[start])
        body = [lines[start]]
        for child in lines[start + 1 :]:
            if re.match(rf"^ {{{base_indent}}}-\s+", child):
                break
            body.append(child)
        return "\n".join(body)
    return ""


def workflow_step_id_block(text: str, step_id: str) -> str:
    lines = text.splitlines()
    id_pattern = re.compile(rf"^\s+(?:-\s*)?id\s*:\s*{re.escape(step_id)}\s*$")
    for index, line in enumerate(lines):
        if not id_pattern.match(line):
            continue
        start = index
        while start > 0 and not re.match(r"^\s*-\s+", lines[start]):
            start -= 1
        base_indent = indent_of(lines[start])
        body = [lines[start]]
        for child in lines[start + 1 :]:
            if re.match(rf"^ {{{base_indent}}}-\s+", child):
                break
            body.append(child)
        return "\n".join(body)
    return ""


def github_app_token_errors(text: str) -> list[str]:
    errors: list[str] = []
    if not re.search(r"(?m)^\s{6}grimoire_app_client_id\s*:", text):
        errors.append("workflow_call must declare grimoire_app_client_id input")
    if not re.search(r"(?ms)^\s{6}GRIMOIRE_APP_PRIVATE_KEY\s*:\s*$.*?^\s{8}required\s*:\s*true\s*$", text):
        errors.append("workflow_call must declare required GRIMOIRE_APP_PRIVATE_KEY secret")

    token_refs = re.findall(r"actions/create-github-app-token@([^\s#]+)", text)
    if len(token_refs) != 1:
        errors.append("workflow must use actions/create-github-app-token exactly once")
    elif not re.fullmatch(r"[0-9A-Fa-f]{40}", token_refs[0]):
        errors.append("actions/create-github-app-token must be pinned to a 40-character commit SHA")

    token_step = workflow_step_id_block(text, APP_TOKEN_STEP_ID)
    if not token_step:
        errors.append(f"workflow missing GitHub App token step id: {APP_TOKEN_STEP_ID}")
    else:
        if APP_TOKEN_ACTION_RE.search(token_step) is None:
            errors.append("grimoire-app-token step must use actions/create-github-app-token pinned to a 40-character commit SHA")
        for snippet in (
            "client-id: ${{ inputs.grimoire_app_client_id }}",
            "private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}",
            "owner: DongwonTTuna-Labs",
        ):
            if snippet not in token_step:
                errors.append(f"grimoire-app-token step missing required snippet: {snippet}")
        if "app-id:" in token_step:
            errors.append("grimoire-app-token step must use client-id, not app-id")

    for step_name, label in (
        ("Checkout trusted control plane", "control-plane checkout"),
        ("Checkout consumer repository as data", "consumer checkout"),
    ):
        step = workflow_step_block(text, step_name)
        if not step:
            errors.append(f"workflow missing {label} step: {step_name}")
        elif f"token: {APP_TOKEN_EXPR}" not in step:
            errors.append(f"{label} must use {APP_TOKEN_EXPR}")

    cast = workflow_step_block(text, "Run cast driver")
    if not cast:
        errors.append("workflow missing Run cast driver step")
    elif f"GRIMOIRE_GITHUB_PAT: {APP_TOKEN_EXPR}" not in cast:
        errors.append(f"Run cast driver must set GRIMOIRE_GITHUB_PAT from {APP_TOKEN_EXPR}")
    return errors


def opencode_runtime_setup_errors(text: str) -> list[str]:
    errors: list[str] = []
    setup = workflow_step_block(text, "Provision opencode runtime")
    if not setup:
        return ["workflow missing opencode runtime provisioning step: Provision opencode runtime"]
    setup_index = text.find("name: Provision opencode runtime")
    preflight_index = text.find("name: Validate opencode runtime")
    if preflight_index == -1:
        errors.append("workflow missing opencode runtime preflight step: Validate opencode runtime")
    elif setup_index > preflight_index:
        errors.append("workflow opencode runtime provisioning must run before Validate opencode runtime")
    for snippet in (
        "id: setup-opencode",
        "python3 control-plane/actions/grimoire/cast/scripts/setup_opencode.py",
    ):
        if snippet not in setup:
            errors.append(f"workflow opencode runtime provisioning missing required snippet: {snippet}")
    for snippet in ("pull_request_target", "secrets: inherit", "ubuntu-latest", "secrets.", "steps.auth.outputs", "GRIMOIRE_PAT", "AI_RELAY_API_KEY", "CF_ACCESS_CLIENT_ID", "CF_ACCESS_CLIENT_SECRET"):
        if snippet in setup:
            errors.append(f"workflow opencode runtime provisioning contains forbidden snippet: {snippet}")
    return errors


def opencode_runtime_preflight_errors(text: str) -> list[str]:
    errors: list[str] = []
    preflight = workflow_step_block(text, "Validate opencode runtime")
    if not preflight:
        return ["workflow missing opencode runtime preflight step: Validate opencode runtime"]
    preflight_index = text.find("name: Validate opencode runtime")
    cast_index = text.find("name: Run cast driver")
    if cast_index == -1:
        errors.append("workflow missing Run cast driver step")
    elif preflight_index > cast_index:
        errors.append("workflow opencode runtime preflight must run before Run cast driver")
    for snippet in (
        "id: opencode",
        "command -v opencode",
        "--version",
        "missing-runtime:opencode-unavailable",
        "runtime-failed:opencode-command-failed",
    ):
        if snippet not in preflight:
            errors.append(f"workflow opencode runtime preflight missing required snippet: {snippet}")
    for snippet in ("GITHUB_PATH", "pull_request_target", "secrets: inherit", "ubuntu-latest"):
        if snippet in preflight:
            errors.append(f"workflow opencode runtime preflight contains forbidden snippet: {snippet}")
    return errors


def run_validate_workflow(args: argparse.Namespace) -> int:
    path = pathlib.Path(args.workflow)
    text = path.read_text(encoding="utf-8")
    errors: list[str] = []
    try:
        block = workflow_input_block(text)
    except ContractError as exc:
        errors.append(str(exc))
        block = ""
    for event in FORBIDDEN_EVENTS:
        if re.search(rf"(?m)^\s*{re.escape(event)}\s*:", text):
            errors.append(f"forbidden event trigger: {event}")
    if "secrets: inherit" in text:
        errors.append("forbidden secrets: inherit")
    if re.search(r"\b(ubuntu|macos|windows)-latest\b", text):
        errors.append("forbidden GitHub-hosted runner fallback")
    for marker in FORBIDDEN_PRIVILEGED_AUTH_MARKERS:
        if marker in text:
            errors.append(f"forbidden privileged auth marker: {marker}")
    errors.extend(github_app_token_errors(text))
    if "./actions/grimoire" in text:
        errors.append("bare ./actions/grimoire path is forbidden inside reusable workflow")
    input_section = re.search(r"(?ms)^\s{6}inputs\s*:\s*(.*?)(?:^\s{6}secrets\s*:|\Z)", text)
    inputs_text = input_section.group(1) if input_section else block
    for name in FORBIDDEN_INPUTS:
        if re.search(rf"(?im)^\s{{6,}}{re.escape(name)}\s*:", inputs_text):
            errors.append("Grimoire must not expose runtime toggles")
    errors.extend(opencode_runtime_setup_errors(text))
    errors.extend(opencode_runtime_preflight_errors(text))
    required_snippets = (
        "permissions: {}",
        "workflow_call:",
        "consumer_repository:",
        "consumer_ref:",
        "pull_request_number:",
        "head_sha:",
        "base_ref:",
        "grimoire_contract_version:",
        "grimoire_app_client_id:",
        "GRIMOIRE_APP_PRIVATE_KEY:",
        "AI_RELAY_API_KEY:",
        "CF_ACCESS_CLIENT_ID:",
        "CF_ACCESS_CLIENT_SECRET:",
        "actions/create-github-app-token@",
        "client-id: ${{ inputs.grimoire_app_client_id }}",
        "private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}",
        "owner: DongwonTTuna-Labs",
        f"GRIMOIRE_GITHUB_PAT: {APP_TOKEN_EXPR}",
        "group: Home Server Runners",
        "labels: dongwontuna-labs-runner",
        "repository: DongwonTTuna-Labs/home-server-infra",
        "ref: main",
        "path: control-plane",
        "path: consumer",
        "persist-credentials: false",
        "./control-plane/actions/grimoire/trusted-controller",
        "./control-plane/actions/grimoire/cast",
        "Provision opencode runtime",
        "control-plane/actions/grimoire/cast/scripts/setup_opencode.py",
        "Settings -> Actions -> General -> Access",
    )
    for snippet in required_snippets:
        if snippet not in text:
            errors.append(f"workflow missing required contract snippet: {snippet}")
    if text.count("./control-plane/actions/grimoire/trusted-controller") != 1:
        errors.append("trusted-controller must be referenced exactly once in the orchestrator")
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("workflow contract ok")
    return 0


def add_common(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--github-output", default="")


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the Grimoire cast driver contract helpers.")
    sub = parser.add_subparsers(dest="command", required=True)

    preflight = sub.add_parser("preflight")
    add_common(preflight)
    preflight.add_argument("--trusted-status-path", required=True)
    preflight.add_argument("--trusted-outcome", default="")
    preflight.add_argument("--trusted-status", default="")
    preflight.add_argument("--trusted-action", default="")
    preflight.add_argument("--model-execution-allowed", default="")
    preflight.add_argument("--write-allowed", default="")
    preflight.add_argument("--commit-allowed", default="")
    preflight.add_argument("--push-allowed", default="")
    preflight.add_argument("--github-mutation-allowed", default="")
    preflight.add_argument("--output", default=".omo/ci/cast-preflight.json")

    issues = sub.add_parser("file-issues")
    add_common(issues)
    issues.add_argument("--design-path", default=".omo/ci/spec-sufficiency.json")
    issues.add_argument("--repository", required=True)
    issues.add_argument("--pr-number", required=True)
    issues.add_argument("--ledger", default=".omo/ci/out-of-scope-issues-ledger.json")
    issues.add_argument("--output", default=".omo/ci/out-of-scope-issues-status.json")

    comment = sub.add_parser("upsert-spec-gap-comment")
    add_common(comment)
    comment.add_argument("--decision", default=".omo/ci/cast-decision.json")
    comment.add_argument("--spec-gap-status", default=".omo/ci/spec-gap-status.json")
    comment.add_argument("--comment-path", default="")
    comment.add_argument("--repository", required=True)
    comment.add_argument("--pr-number", required=True)
    comment.add_argument("--github-mutation-allowed", default="false")
    comment.add_argument("--output", default=".omo/ci/spec-gap-comment-upsert.json")

    boulder = sub.add_parser("boulder")
    add_common(boulder)
    boulder.add_argument("--fix-status", default=".omo/ci/fix-status.json")
    boulder.add_argument("--timeout-minutes", type=int, default=90)
    boulder.add_argument("--output", default=".omo/boulder.json")

    decide = sub.add_parser("decide")
    add_common(decide)
    decide.add_argument("--preflight-status", default=".omo/ci/cast-preflight.json")
    decide.add_argument("--review-status", default=".omo/ci/review-findings.json")
    decide.add_argument("--review-outcome", default="")
    decide.add_argument("--design-status", default=".omo/ci/spec-sufficiency.json")
    decide.add_argument("--issue-status", default=".omo/ci/out-of-scope-issues-status.json")
    decide.add_argument("--spec-gap-status", default=".omo/ci/spec-gap-status.json")
    decide.add_argument("--fix-status", default=".omo/ci/fix-status.json")
    decide.add_argument("--fix-outcome", default="")
    decide.add_argument("--boulder-status", default=".omo/boulder.json")
    decide.add_argument("--verdict-status", default=".omo/grimoire/verdict.json")
    decide.add_argument("--verify-outcome", default="")
    decide.add_argument("--output", default=".omo/ci/cast-decision.json")

    push = sub.add_parser("push")
    add_common(push)
    push.add_argument("--decision", default=".omo/ci/cast-decision.json")
    push.add_argument("--repository", required=True)
    push.add_argument("--consumer-ref", required=True)
    push.add_argument("--head-sha", default="")
    push.add_argument("--output", default=".omo/ci/cast-push-status.json")

    complete = sub.add_parser("complete")
    add_common(complete)
    complete.add_argument("--decision", default=".omo/ci/cast-decision.json")
    complete.add_argument("--push-status", default=".omo/ci/cast-push-status.json")
    complete.add_argument("--output", default=".omo/ci/cast-final-status.json")

    validate = sub.add_parser("validate-workflow")
    validate.add_argument("--workflow", required=True)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    if args.command == "preflight":
        return run_preflight(args)
    if args.command == "file-issues":
        return run_file_issues(args)
    if args.command == "upsert-spec-gap-comment":
        return run_upsert_spec_gap_comment(args)
    if args.command == "boulder":
        return run_boulder(args)
    if args.command == "decide":
        return run_decide(args)
    if args.command == "push":
        return run_push(args)
    if args.command == "complete":
        return run_complete(args)
    if args.command == "validate-workflow":
        return run_validate_workflow(args)
    raise ContractError(f"unsupported command: {args.command}")


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
