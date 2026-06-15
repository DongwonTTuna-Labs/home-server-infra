#!/usr/bin/env python3
# pyright: reportAny=false, reportExplicitAny=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from typing import Any

STAGE = "grimoire-labels"
DEFAULT_GITHUB_API = "https://api.github.com"
LABELS = {
    "casting": {"name": "🔮 Casting…", "color": "#7c3aed", "description": "Grimoire review/autofix loop is running."},
    "cast": {"name": "✨ Cast", "color": "#10b981", "description": "Grimoire review/autofix loop completed cleanly."},
    "fizzled": {"name": "💨 Fizzled", "color": "#6b7280", "description": "Grimoire review/autofix loop halted or failed closed."},
}
MANAGED = [LABELS["casting"]["name"], LABELS["cast"]["name"], LABELS["fizzled"]["name"]]
TOKEN_PATTERNS = (
    re.compile(r"github_pat_[A-Za-z0-9_]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(r"sk-[A-Za-z0-9_-]{20,}"),
    re.compile(r"(?i)bearer\s+[A-Za-z0-9._~-]{16,}"),
    re.compile(r"(?i)(token|secret|password|api[_-]?key)\s*[:=]\s*\S+"),
)


class RemoteLabelError(Exception):
    pass


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def bool_text(value: Any) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def as_bool(value: Any) -> bool:
    if isinstance(value, bool):
        return value
    return str(value or "").strip().lower() in {"true", "1", "yes"}


def sanitize(value: Any, max_len: int = 500, secrets: tuple[str, ...] = ()) -> str:
    text = str(value or "").replace("\r", " ").strip()
    text = re.sub(r"\s+", " ", text)
    for secret in secrets:
        if secret:
            text = text.replace(secret, "[REDACTED]")
    for pattern in TOKEN_PATTERNS:
        text = pattern.sub("[REDACTED]", text)
    if len(text) > max_len:
        return text[: max_len - 1].rstrip() + "…"
    return text


def resolve_path(raw: str, workspace: pathlib.Path) -> pathlib.Path:
    path = pathlib.Path(raw)
    if path.is_absolute():
        return path
    return workspace / path


def read_labels(path: pathlib.Path) -> list[str]:
    if not path.exists():
        return []
    labels: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        label = line.strip()
        if label and label not in labels:
            labels.append(label)
    return labels


def write_labels(path: pathlib.Path, labels: list[str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(labels) + ("\n" if labels else ""), encoding="utf-8")


def transition_labels(current: list[str], transition: str) -> tuple[list[str], list[dict[str, str]], list[str]]:
    final = list(current)
    operations: list[dict[str, str]] = []
    notes: list[str] = []

    def remove(label: str, reason: str) -> None:
        if label in final:
            final.remove(label)
            operations.append({"action": "remove", "label": label, "reason": reason})
        else:
            notes.append(f"remove skipped for absent label: {label}")

    def add(label: str, reason: str) -> None:
        if label in final:
            notes.append(f"add skipped for existing label: {label}")
        else:
            final.append(label)
            operations.append({"action": "add", "label": label, "reason": reason})

    if transition == "running":
        if LABELS["cast"]["name"] in final or LABELS["fizzled"]["name"] in final:
            notes.append("running skipped because a terminal grimoire label is already present")
        elif LABELS["casting"]["name"] in final:
            notes.append("running skipped because Casting is already present")
        else:
            add(LABELS["casting"]["name"], "running transition adds Casting")
    elif transition == "done":
        remove(LABELS["casting"]["name"], "done transition removes running label")
        remove(LABELS["fizzled"]["name"], "done transition removes halted label")
        add(LABELS["cast"]["name"], "done transition adds Cast")
    elif transition == "fizzled":
        remove(LABELS["casting"]["name"], "fizzled transition removes running label")
        remove(LABELS["cast"]["name"], "fizzled transition removes success label")
        add(LABELS["fizzled"]["name"], "fizzled transition adds Fizzled")
    else:
        raise ValueError(f"unsupported transition: {transition}")
    return final, operations, notes


def remote_transition_operations(current: list[str], transition: str) -> list[dict[str, str]]:
    def remove(label: str, reason: str) -> dict[str, str]:
        return {"action": "remove", "label": label, "reason": reason}

    def add(label: str, reason: str) -> dict[str, str]:
        return {"action": "add", "label": label, "reason": reason}

    if transition == "running":
        if LABELS["cast"]["name"] in current or LABELS["fizzled"]["name"] in current or LABELS["casting"]["name"] in current:
            return []
        return [add(LABELS["casting"]["name"], "running transition adds Casting")]
    if transition == "done":
        return [
            remove(LABELS["casting"]["name"], "done transition removes running label"),
            remove(LABELS["fizzled"]["name"], "done transition removes halted label"),
            add(LABELS["cast"]["name"], "done transition adds Cast"),
        ]
    if transition == "fizzled":
        return [
            remove(LABELS["casting"]["name"], "fizzled transition removes running label"),
            remove(LABELS["cast"]["name"], "fizzled transition removes success label"),
            add(LABELS["fizzled"]["name"], "fizzled transition adds Fizzled"),
        ]
    raise ValueError(f"unsupported transition: {transition}")


def write_github_output(path: str | None, values: dict[str, object]) -> None:
    if not path:
        return
    with pathlib.Path(path).open("a", encoding="utf-8") as handle:
        for key, value in values.items():
            handle.write(f"{key}={bool_text(value)}\n")


def is_remote_repository(repository: str) -> bool:
    return bool(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository or ""))


def valid_issue_number(value: str) -> bool:
    text = str(value or "").strip()
    return text.isdigit() and int(text) > 0


def remote_context(args: argparse.Namespace) -> tuple[bool, str]:
    if not as_bool(args.remote_apply):
        return False, "disabled"
    if not str(args.token or "").strip():
        return False, "missing-token"
    if not is_remote_repository(args.repository):
        return False, "local-repository"
    if not valid_issue_number(str(args.pr_number)):
        return False, "missing-pr-number"
    return True, "enabled"


def github_request(method: str, path: str, token: str, payload: dict[str, Any] | None, api_url: str) -> Any:
    data = None if payload is None else json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        api_url.rstrip("/") + path,
        data=data,
        method=method,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "X-GitHub-Api-Version": "2022-11-28",
            "Content-Type": "application/json",
            "User-Agent": "grimoire-labels",
        },
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        body = response.read().decode("utf-8")
        return json.loads(body) if body else {}


def remote_error(operation: str, label: str, exc: BaseException, token: str) -> RemoteLabelError:
    if isinstance(exc, urllib.error.HTTPError):
        message = f"GitHub label {operation} failed for {label}: HTTP {exc.code} {exc.reason}"
    elif isinstance(exc, urllib.error.URLError):
        message = f"GitHub label {operation} failed for {label}: {exc.reason}"
    else:
        message = f"GitHub label {operation} failed for {label}: {exc}"
    return RemoteLabelError(sanitize(message, secrets=(token,)))


def apply_remote_operations(args: argparse.Namespace, operations: list[dict[str, str]]) -> tuple[bool, list[dict[str, Any]]]:
    repository = str(args.repository).strip()
    pr_number = str(args.pr_number).strip()
    token = str(args.token or "")
    api_url = str(args.github_api_url or DEFAULT_GITHUB_API)
    attempted = False
    results: list[dict[str, Any]] = []
    add_labels: list[str] = []

    for operation in operations:
        action = operation["action"]
        label = operation["label"]
        if action == "add":
            add_labels.append(label)
            continue
        if action != "remove":
            raise RemoteLabelError(f"unsupported GitHub label operation: {sanitize(action)}")
        attempted = True
        encoded = urllib.parse.quote(label, safe="")
        path = f"/repos/{repository}/issues/{pr_number}/labels/{encoded}"
        try:
            github_request("DELETE", path, token, None, api_url)
            results.append({"action": "remove", "label": label, "status": "applied"})
        except urllib.error.HTTPError as exc:
            if exc.code == 404:
                results.append({"action": "remove", "label": label, "status": "missing", "http_status": 404})
                continue
            raise remote_error("remove", label, exc, token) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise remote_error("remove", label, exc, token) from exc

    if add_labels:
        attempted = True
        path = f"/repos/{repository}/issues/{pr_number}/labels"
        try:
            github_request("POST", path, token, {"labels": add_labels}, api_url)
            results.extend({"action": "add", "label": label, "status": "applied"} for label in add_labels)
        except urllib.error.HTTPError as exc:
            raise remote_error("add", ", ".join(add_labels), exc, token) from exc
        except (urllib.error.URLError, TimeoutError, OSError) as exc:
            raise remote_error("add", ", ".join(add_labels), exc, token) from exc

    return attempted, results


def run(args: argparse.Namespace) -> int:
    workspace = pathlib.Path(args.consumer_workspace).resolve()
    state_file = resolve_path(args.state_file, workspace)
    state_output = resolve_path(args.state_output or args.state_file, workspace)
    status_output = resolve_path(args.status_output, workspace)
    current = read_labels(state_file)
    final, operations, notes = transition_labels(current, args.transition)
    write_labels(state_output, final)

    remote_operations = remote_transition_operations(current, args.transition)
    remote_enabled, remote_skip_reason = remote_context(args)
    remote_attempted = False
    remote_results: list[dict[str, Any]] = []
    remote_status = "skipped-" + remote_skip_reason
    blocked_reason = ""
    exit_code = 0

    if remote_enabled:
        if remote_operations:
            try:
                remote_attempted, remote_results = apply_remote_operations(args, remote_operations)
                remote_status = "applied" if remote_attempted else "skipped-no-operations"
            except RemoteLabelError as exc:
                remote_attempted = True
                remote_status = "failed"
                blocked_reason = sanitize(exc, secrets=(str(args.token or ""),))
                notes.append(f"remote apply failed closed: {blocked_reason}")
                exit_code = 1
        else:
            remote_status = "skipped-no-operations"
            notes.append("remote apply skipped because transition produced no managed GitHub label operations")
    else:
        notes.append(f"remote apply skipped: {remote_skip_reason}")

    report = {
        "schema_version": 1,
        "stage": STAGE,
        "generated_at": utc_now(),
        "status": "blocked" if exit_code else "ok",
        "transition": args.transition,
        "mode": "github-issues-labels-api" if remote_enabled else "local-state-file",
        "repo": args.repository,
        "pr_number": args.pr_number,
        "uses_default_actions_token": False,
        "labels_are_display_only": not remote_enabled,
        "durable_loop_state_source": False,
        "managed_labels": list(LABELS.values()),
        "unrelated_labels_preserved": sorted(label for label in final if label not in MANAGED),
        "current_labels": current,
        "final_labels": final,
        "operations": operations,
        "operation_count": len(operations),
        "changed": bool(operations),
        "github_pr_label_mutation_requested": as_bool(args.remote_apply),
        "github_pr_label_mutation_enabled": remote_enabled,
        "github_pr_label_mutation_attempted": remote_attempted,
        "github_api_url": str(args.github_api_url or DEFAULT_GITHUB_API),
        "github_label_operations": remote_operations if remote_enabled else [],
        "github_label_operation_count": len(remote_operations) if remote_enabled else 0,
        "github_label_results": remote_results,
        "remote_apply_status": remote_status,
        "remote_apply_skip_reason": "" if remote_enabled else remote_skip_reason,
        "token_source": "pat-input" if str(args.token or "").strip() else "none",
        "notes": notes,
    }
    if blocked_reason:
        report["blocked_reason"] = blocked_reason
    status_output.parent.mkdir(parents=True, exist_ok=True)
    status_output.write_text(json.dumps(report, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    write_github_output(
        args.github_output,
        {
            "changed": bool(operations),
            "operation_count": len(operations),
            "status_path": str(status_output),
            "state_output": str(state_output),
            "github_pr_label_mutation_attempted": remote_attempted,
            "remote_apply_status": remote_status,
        },
    )
    print(f"{STAGE}: transition={args.transition} operations={len(operations)} remote={remote_status} status={status_output}")
    return exit_code


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Apply idempotent Grimoire label transitions in local state-file mode and optional trusted GitHub remote-apply mode.")
    parser.add_argument("transition", choices=["running", "done", "fizzled"])
    parser.add_argument("--consumer-workspace", default=os.environ.get("GITHUB_WORKSPACE", "."))
    parser.add_argument("--state-file", default=".omo/ci/grimoire-label-state.txt")
    parser.add_argument("--state-output", default="")
    parser.add_argument("--status-output", default=".omo/ci/grimoire-label-status.json")
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY", "local-consumer"))
    parser.add_argument("--pr-number", default=os.environ.get("GRIMOIRE_PR_NUMBER", "0"))
    parser.add_argument("--remote-apply", default=os.environ.get("GRIMOIRE_LABEL_REMOTE_APPLY", "false"))
    parser.add_argument("--token", default=os.environ.get("GRIMOIRE_LABEL_TOKEN", os.environ.get("GRIMOIRE_GITHUB_PAT", "")))
    parser.add_argument("--github-api-url", default=os.environ.get("GITHUB_API_URL", DEFAULT_GITHUB_API))
    parser.add_argument("--github-output", default="")
    return parser.parse_args(argv)


if __name__ == "__main__":
    raise SystemExit(run(parse_args(sys.argv[1:])))
