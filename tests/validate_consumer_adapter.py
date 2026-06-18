# pyright: reportAny=false, reportUnknownMemberType=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


EXPECTED_PR_TYPES = ["opened", "ready_for_review", "synchronize", "reopened", "unlabeled"]
EXPECTED_WITH_KEYS = {"consumer_repository", "consumer_ref", "pull_request_number", "head_sha", "base_ref", "grimoire_app_client_id"}
EXPECTED_SECRETS = {"GRIMOIRE_APP_PRIVATE_KEY", "AI_RELAY_API_KEY", "CF_ACCESS_CLIENT_ID", "CF_ACCESS_CLIENT_SECRET"}
FORBIDDEN_EVENTS = {"workflow_dispatch", "pull_request_target", "push"}
FORBIDDEN_RUNTIME_KEYS = {"mode", "dry_run", "dry-run", "allow_live", "allow-live", "simulate", "simulation"}
STOP_LABEL = "grimoire:disabled"
SPEC_NEEDED_LABEL = "📋 Spec Needed"


class ContractError(AssertionError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ContractError(f"missing consumer workflow: {path}") from exc


def indent_of(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def section_body(text: str, key: str, indent: int) -> str:
    lines = text.splitlines()
    prefix = " " * indent
    for index, line in enumerate(lines):
        if line.startswith(prefix) and indent_of(line) == indent and re.match(rf"^{prefix}{re.escape(key)}\s*:", line):
            body: list[str] = []
            for child in lines[index + 1 :]:
                if child.strip() and indent_of(child) <= indent:
                    break
                body.append(child)
            return "\n".join(body)
    raise ContractError(f"missing YAML section: {' ' * indent}{key}:")


def keys_at_indent(text: str, indent: int) -> set[str]:
    return set(re.findall(rf"(?m)^ {{{indent}}}([A-Za-z0-9_-]+)\s*:", text))


def parse_mapping(block: str, indent: int) -> dict[str, str]:
    values: dict[str, str] = {}
    pattern = re.compile(rf"^ {{{indent}}}([A-Za-z0-9_-]+)\s*:\s*(.*?)\s*$", re.MULTILINE)
    for match in pattern.finditer(block):
        values[match.group(1)] = match.group(2).strip()
    return values


def mapping_value(block: str, key: str, indent: int) -> str:
    lines = block.splitlines()
    prefix = " " * indent
    block_markers = {">", ">-", ">+", "|", "|-", "|+"}
    for index, line in enumerate(lines):
        match = re.match(rf"^{prefix}{re.escape(key)}\s*:\s*(.*?)\s*$", line)
        if match is None:
            continue
        value = match.group(1).strip()
        if value and value not in block_markers:
            return value
        parts: list[str] = []
        for child in lines[index + 1 :]:
            if child.strip() and indent_of(child) <= indent:
                break
            if child.strip():
                parts.append(child.strip())
        return " ".join(parts)
    raise ContractError(f"missing YAML value: {' ' * indent}{key}:")


def normalize_guard_expression(value: str) -> str:
    expression = value.replace("${{", " ").replace("}}", " ")
    return re.sub(r"\s+", " ", expression).strip()


def has_spec_needed_unlabeled_guard(guard: str) -> bool:
    normalized = normalize_guard_expression(guard)
    label_literal = rf"['\"]{re.escape(SPEC_NEEDED_LABEL)}['\"]"
    action_not_unlabeled = r"github\.event\.action\s*!=\s*['\"]unlabeled['\"]"
    label_is_spec_needed = rf"github\.event\.label\.name\s*==\s*{label_literal}"
    unlabeled_guard = rf"(?:{action_not_unlabeled}\s*\|\|\s*{label_is_spec_needed}|{label_is_spec_needed}\s*\|\|\s*{action_not_unlabeled})"
    if re.fullmatch(rf"\(?\s*{unlabeled_guard}\s*\)?", normalized):
        return True
    return re.search(rf"\(\s*{unlabeled_guard}\s*\)", normalized) is not None


def assert_pull_request_trigger(text: str) -> list[str]:
    on_block = section_body(text, "on", 0)
    events = keys_at_indent(on_block, 2)
    require(events == {"pull_request"}, f"consumer workflow must expose only pull_request, got {sorted(events)}")
    forbidden = events & FORBIDDEN_EVENTS
    require(not forbidden, "forbidden consumer workflow events: " + ", ".join(sorted(forbidden)))

    pull_request = section_body(text, "pull_request", 2)
    inline = re.search(r"(?m)^    types\s*:\s*\[(.*?)\]\s*$", pull_request)
    if inline:
        types = [item.strip().strip("'\"") for item in inline.group(1).split(",") if item.strip()]
    else:
        types_block = section_body(text, "types", 4)
        types = []
        for line in types_block.splitlines():
            match = re.match(r"^\s*-\s*([A-Za-z0-9_-]+)\s*$", line)
            if match:
                types.append(match.group(1))
    require(types == EXPECTED_PR_TYPES, f"pull_request types drifted: {types}")
    return types


def find_reusable_job_block(text: str, expected_reusable_repo: str) -> tuple[str, str, str]:
    pattern = re.compile(
        rf"(?m)^\s{{4}}uses\s*:\s*({re.escape(expected_reusable_repo)}/\.github/workflows/grimoire-control-plane\.ya?ml)@([^\s#]+)\s*$"
    )
    match = pattern.search(text)
    require(match is not None, "consumer job must call the expected Grimoire reusable workflow")
    assert match is not None
    lines = text.splitlines()
    char_index = match.start()
    line_index = text[:char_index].count("\n")
    start = line_index
    while start > 0 and not re.match(r"^  [A-Za-z0-9_-]+\s*:", lines[start]):
        start -= 1
    job_name = lines[start].strip().rstrip(":")
    body = [lines[start]]
    for child in lines[start + 1 :]:
        if child.strip() and indent_of(child) <= 2:
            break
        body.append(child)
    return job_name, "\n".join(body), match.group(2)


def assert_job_guard(job_block: str, pr_types: list[str]) -> None:
    try:
        guard = mapping_value(job_block, "if", 4)
    except ContractError as exc:
        raise ContractError(f"consumer job must have a job-level Ready/non-{STOP_LABEL} guard") from exc
    require("pull_request" in guard and "draft" in guard and "false" in guard, "consumer guard must skip draft PRs")
    require(STOP_LABEL in guard and "contains" in guard and "!" in guard, f"consumer guard must skip the {STOP_LABEL} stop label")
    require("LGTM" not in guard and "codex:lgtm" not in guard, "consumer guard must not use legacy Codex/LGTM stop labels")
    if "unlabeled" in pr_types:
        require(
            has_spec_needed_unlabeled_guard(guard),
            f"consumer guard must allow unlabeled reruns only for {SPEC_NEEDED_LABEL}",
        )


def assert_reusable_call(job_block: str, ref: str, expected_ref: str) -> None:
    require(ref == expected_ref, f"consumer workflow must call @{expected_ref}, got @{ref}")
    with_block = section_body(job_block, "with", 4)
    with_values = parse_mapping(with_block, 6)
    with_keys = set(with_values)
    require(with_keys == EXPECTED_WITH_KEYS, f"consumer with keys must be only repo/PR metadata plus App client ID, got {sorted(with_keys)}")
    forbidden = with_keys & FORBIDDEN_RUNTIME_KEYS
    require(not forbidden, "consumer with block must not pass runtime toggles: " + ", ".join(sorted(forbidden)))
    required_value_markers = {
        "consumer_repository": ("github.repository",),
        "consumer_ref": ("github.head_ref", "github.event.pull_request.head.ref"),
        "pull_request_number": ("github.event.pull_request.number",),
        "head_sha": ("github.event.pull_request.head.sha",),
        "base_ref": ("github.base_ref", "github.event.pull_request.base.ref"),
        "grimoire_app_client_id": ("Iv23liFL1dDHmU06FLSF", "vars.GRIMOIRE_APP_CLIENT_ID"),
    }
    for key, markers in required_value_markers.items():
        value = with_values.get(key, "")
        if key == "grimoire_app_client_id":
            require("secrets." not in value, "consumer with.grimoire_app_client_id must be a non-secret App client ID")
            require(any(marker in value for marker in markers), "consumer with.grimoire_app_client_id must use the Grimoire App client ID")
            continue
        require(any(marker in value for marker in markers), f"consumer with.{key} must use GitHub PR metadata")


def assert_secrets(job_block: str, text: str) -> None:
    require(not re.search(r"(?m)^\s*secrets\s*:\s*inherit\s*$", text), "consumer workflow must not use secrets: inherit")
    require("GITHUB_TOKEN" not in text and "github.token" not in text, "consumer workflow must not map Grimoire auth to GITHUB_TOKEN")
    require("GRIMOIRE_PAT" not in text, "consumer workflow must not use legacy GRIMOIRE_PAT auth")
    secrets_block = section_body(job_block, "secrets", 4)
    secret_values = parse_mapping(secrets_block, 6)
    require(set(secret_values) == EXPECTED_SECRETS, f"consumer must explicitly map only named secrets, got {sorted(secret_values)}")
    for secret in EXPECTED_SECRETS:
        require(secret_values[secret] == f"${{{{ secrets.{secret} }}}}", f"consumer secret {secret} must map explicitly to secrets.{secret}")


def assert_no_forbidden_runtime_shapes(text: str) -> None:
    require(re.search(r"(?m)^permissions\s*:\s*\{\}\s*$", text) is not None, "consumer workflow must keep top-level permissions: {}")
    for event in FORBIDDEN_EVENTS:
        require(not re.search(rf"(?m)^\s{{2}}{re.escape(event)}\s*:", text), f"forbidden consumer event: {event}")
    for key in FORBIDDEN_RUNTIME_KEYS:
        require(not re.search(rf"(?m)^\s{{4,}}{re.escape(key)}\s*:", text), f"forbidden consumer runtime toggle: {key}")


def validate(workflow: Path, expected_reusable_repo: str, expected_ref: str) -> None:
    text = read_text(workflow)
    pr_types = assert_pull_request_trigger(text)
    assert_no_forbidden_runtime_shapes(text)
    _job_name, job_block, ref = find_reusable_job_block(text, expected_reusable_repo)
    assert_job_guard(job_block, pr_types)
    assert_reusable_call(job_block, ref, expected_ref)
    assert_secrets(job_block, text)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate a thin Grimoire consumer adapter workflow.")
    parser.add_argument("--workflow", required=True, type=Path)
    parser.add_argument("--expected-reusable-repo", required=True)
    parser.add_argument("--expected-ref", required=True)
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        validate(args.workflow.resolve(), args.expected_reusable_repo, args.expected_ref)
    except ContractError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print("consumer adapter ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
