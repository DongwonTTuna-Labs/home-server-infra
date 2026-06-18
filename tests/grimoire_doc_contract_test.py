from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import cast


EXPECTED_STAGES = (
    "trusted-controller",
    "review",
    "design",
    "spec-gap",
    "fix",
    "verify",
    "labels",
    "cast",
)

EXPECTED_PATHS = (
    ".github/workflows/grimoire-control-plane.yml",
    "actions/grimoire/trusted-controller/action.yml",
    "actions/grimoire/review/action.yml",
    "actions/grimoire/design/action.yml",
    "actions/grimoire/spec-gap/action.yml",
    "actions/grimoire/fix/action.yml",
    "actions/grimoire/verify/action.yml",
    "actions/grimoire/labels/action.yml",
    "actions/grimoire/cast/action.yml",
    "actions/grimoire/<stage>/scripts/*",
    "config/grimoire/opencode.json",
    "config/grimoire/oh-my-openagent.jsonc",
    "schemas/grimoire-workflow-call.v1.schema.json",
    "schemas/grimoire-scope.v1.schema.json",
    "tests/grimoire_workflow_contract_test.py",
    "tests/grimoire_action_contract_test.py",
    "tests/grimoire_stage_contract_test.py",
    "tests/grimoire_secret_hygiene_test.py",
    "tests/grimoire_doc_contract_test.py",
    "tests/validate_consumer_adapter.py",
    "docs/grimoire-reusable.md",
    "docs/decisions/grimoire-reusable-control-plane.md",
    "docs/decisions/grimoire-app-auth.md",
    "docs/decisions/grimoire-advisory-failure-separation.md",
    "docs/releases/grimoire-reusable-control-plane-v1.md",
    "docs/releases/grimoire-reusable-control-plane-v1-advisory-failure.md",
    "docs/releases/grimoire-app-auth-v1.md",
)

ADR_HEADINGS = (
    "# ADR: Grimoire Reusable Control Plane",
    "## Status",
    "## Context",
    "## Source Of Truth",
    "## Recovered Stage Map",
    "## Package Boundary",
    "## Consumer Policy",
    "## Security And Auth",
    "## Scope Guard",
    "## Runtime Policy",
    "## Consequences",
)

DOC_HEADINGS = (
    "# Grimoire Reusable Control Plane",
    "## Source Of Truth",
    "## Recovered Stage Map",
    "## Outcome Taxonomy",
    "## Visual Architecture",
    "## Package Path Map",
    "## Consumer Policy",
    "## Security And Auth",
    "## Scope Guard",
    "## Display Labels",
    "## Runtime Policy",
    "## Release Notes",
    "## Non-Goals",
)

RELEASE_NOTES_PATH = "docs/releases/grimoire-reusable-control-plane-v1.md"
APP_AUTH_ADR_PATH = "docs/decisions/grimoire-app-auth.md"
APP_AUTH_RELEASE_NOTES_PATH = "docs/releases/grimoire-app-auth-v1.md"
ADVISORY_ADR_PATH = "docs/decisions/grimoire-advisory-failure-separation.md"
ADVISORY_RELEASE_NOTES_PATH = "docs/releases/grimoire-reusable-control-plane-v1-advisory-failure.md"

RELEASE_NOTES_REQUIRED_PHRASES = (
    "# Release Notes: Grimoire Reusable Control Plane v1",
    "not a Codex rollback",
    "PR #53",
    "No runtime simulation input",
    "No separate manual Grimoire workflow",
    "Secret hygiene tests",
    "Private consumers can call the reusable workflow only after a maintainer enables access",
    "Consumer repositories should migrate to a thin caller workflow",
    "GitHub App installation-token auth",
    APP_AUTH_ADR_PATH,
    APP_AUTH_RELEASE_NOTES_PATH,
    "GRIMOIRE_APP_PRIVATE_KEY",
    "grimoire_app_client_id",
    "pull_request.unlabeled",
)

APP_AUTH_ADR_HEADINGS = (
    "# ADR: Grimoire GitHub App Auth",
    "## Status",
    "## Context",
    "## Decision",
    "## Alternatives Considered",
    "## Consequences",
    "## Security Rationale",
    "## Migration And Operational Notes",
    "## Validation",
)

APP_AUTH_ADR_REQUIRED_PHRASES = (
    "GitHub App installation token",
    "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349",
    "client-id: ${{ inputs.grimoire_app_client_id }}",
    "private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}",
    "GRIMOIRE_APP_PRIVATE_KEY",
    "grimoire_app_client_id",
    "not a live PAT fallback",
    "token action is SHA-pinned",
    "pull_request.unlabeled",
    "github.event.label.name == '📋 Spec Needed'",
    "<!-- grimoire-spec-gap -->",
    "doesn't claim production rollout, org-wide secret rollout, or observed cross-repo PR-event execution",
)

APP_AUTH_RELEASE_NOTES_HEADINGS = (
    "# Release Notes: Grimoire App Auth v1",
    "## Scope",
    "## Changed",
    "## Migration Checklist",
    "## Spec Needed Re-Review UX",
    "## Security Notes",
    "## Validation Evidence",
    "## Limitations And Follow-Ups",
)

APP_AUTH_RELEASE_NOTES_REQUIRED_PHRASES = (
    "legacy PAT guidance to organization GitHub App installation-token auth",
    "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349",
    "client-id: ${{ inputs.grimoire_app_client_id }}",
    "private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}",
    "GRIMOIRE_APP_PRIVATE_KEY",
    "grimoire_app_client_id",
    "no live legacy PAT fallback",
    "pull_request.unlabeled",
    "github.event.label.name == '📋 Spec Needed'",
    "<!-- grimoire-spec-gap -->",
    "No production rollout claim",
    "No observed cross-repo PR-event execution claim",
    "No claim that organization-level secret or variable rollout is complete",
)

ADVISORY_ADR_HEADINGS = (
    "# ADR: Grimoire Advisory And Failure Separation",
    "## Status",
    "## Context",
    "## Decision",
    "## Alternatives",
    "## Backward-Safety",
    "## Consequences",
)

ADVISORY_ADR_REQUIRED_PHRASES = (
    "success",
    "neutral",
    "failure",
    "advisory",
    "📋 Spec Needed",
    ".omo/grimoire/scope.yml",
    "spec-needed",
    "no_code_or_push_action=true",
    "spec-gap-halt",
    "conclusion=\"neutral\"",
    "no separate neutral check-run",
)

ADVISORY_RELEASE_NOTES_REQUIRED_PHRASES = (
    "# Release Notes: Grimoire Reusable Control Plane v1 Advisory Failure Separation",
    "v1 minor behavior change",
    "No consumer workflow action is required",
    "success",
    "neutral",
    "failure",
    "advisory",
    "📋 Spec Needed",
    ".omo/grimoire/scope.yml",
    "spec-needed",
    "no_code_or_push_action=true",
    "cast_driver/labels/action/docs commits",
    "No separate neutral check-run",
)

REQUIRED_PHRASES = (
    "PR #53",
    "8ed807f6b6d3676b001164dc2116bf87f117d69b",
    "not a Codex rollback",
    "Grimoire opencode and OMO relocation",
    "GitHub App installation-token auth",
    "GitHub App installation token",
    "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349",
    "SHA-pinned App-token action",
    "GRIMOIRE_APP_PRIVATE_KEY",
    "grimoire_app_client_id",
    "GRIMOIRE_GITHUB_PAT",
    "isn't a PAT fallback",
    "CODEX_LOOP_PAT",
    "@main",
    "private consumer policy",
    "secrets: inherit",
    "pull_request_target",
    "GITHUB_TOKEN",
    "no runtime simulation",
    "no separate manual Grimoire workflow",
    "OpenSpec and OMO",
    "out-of-scope findings are filed",
    "dedup fingerprint",
    "Issues-only",
    "Home Server Runners",
    "dongwontuna-labs-runner",
    "grimoire-consumer-workflow:recommended:start",
    "consumer_repository",
    "consumer_ref",
    "pull_request_number",
    "head_sha",
    "base_ref",
    "AI_RELAY_API_KEY",
    "CF_ACCESS_CLIENT_ID",
    "CF_ACCESS_CLIENT_SECRET",
    "CF-Access-Client-Id",
    "CF-Access-Client-Secret",
    "Settings, Actions, General, Access",
    "private reusable workflows and actions",
    "Helper files under `actions/grimoire/<stage>/scripts/` are action-local implementation details",
    "pull_request.synchronize",
    "pull_request.unlabeled",
    "github.event.label.name == '📋 Spec Needed'",
    "<!-- grimoire-spec-gap -->",
    RELEASE_NOTES_PATH,
    APP_AUTH_ADR_PATH,
    APP_AUTH_RELEASE_NOTES_PATH,
    ADVISORY_ADR_PATH,
    ADVISORY_RELEASE_NOTES_PATH,
    "success",
    "neutral",
    "failure",
    "advisory",
    "📋 Spec Needed",
    ".omo/grimoire/scope.yml",
    "spec-needed",
    "no_code_or_push_action=true",
    "spec-gap-halt",
    "conclusion=\"neutral\"",
)

NON_MAIN_REF_PATTERN = re.compile(
    r"DongwonTTuna-Labs/home-server-infra/\.github/workflows/grimoire-control-plane\.ya?ml@([^\s`)\]}]+)"
)

INVALID_LABEL_PATTERN = re.compile(r"\binvalid example\b|\bdo not copy\b", re.IGNORECASE)


class ContractError(AssertionError):
    pass


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ContractError(f"missing required file: {path}") from exc


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def assert_headings(text: str, headings: tuple[str, ...], label: str) -> None:
    for heading in headings:
        require(heading in text, f"{label} missing heading: {heading}")


def assert_required_phrases(combined_text: str) -> None:
    for phrase in REQUIRED_PHRASES:
        require(phrase in combined_text, f"docs missing required phrase: {phrase}")


def assert_stage_map(combined_text: str) -> None:
    positions: list[int] = []
    for stage in EXPECTED_STAGES:
        match = re.search(rf"`{re.escape(stage)}`", combined_text)
        if match is None:
            raise ContractError(f"missing recovered stage: {stage}")
        positions.append(match.start())
    require(positions == sorted(positions), "recovered stages must appear in order")
    print("stage map ok")


def assert_path_map(combined_text: str) -> None:
    for path in EXPECTED_PATHS:
        require(path in combined_text, f"missing package path: {path}")


def has_invalid_context(lines: list[str], line_index: int) -> bool:
    start = max(0, line_index - 8)
    context = "\n".join(lines[start : line_index + 1])
    return bool(INVALID_LABEL_PATTERN.search(context))


def assert_main_consumer_policy(combined_text: str) -> None:
    require("grimoire-control-plane.yml@main" in combined_text, "valid consumer example must use @main")
    lines = combined_text.splitlines()
    for index, line in enumerate(lines):
        for match in NON_MAIN_REF_PATTERN.finditer(line):
            ref = match.group(1).strip()
            if ref == "main":
                continue
            require(
                has_invalid_context(lines, index),
                "Grimoire consumers must use @main",
            )


def assert_scope_guard(combined_text: str) -> None:
    required = (
        "design` is the scope authority",
        "in-scope and out-of-scope",
        "right after design",
        "stable dedup fingerprint",
        "Issues-only",
        "must not fix out-of-scope findings",
    )
    for phrase in required:
        require(phrase in combined_text, f"scope guard missing: {phrase}")
    print("scope guard ok")


def recommended_snippet(docs_text: str) -> str:
    start = "<!-- grimoire-consumer-workflow:recommended:start -->"
    end = "<!-- grimoire-consumer-workflow:recommended:end -->"
    start_index = docs_text.find(start)
    end_index = docs_text.find(end)
    require(start_index != -1 and end_index != -1 and start_index < end_index, "recommended consumer workflow markers missing")
    block = docs_text[start_index:end_index]
    match = re.search(r"```yaml\n(.*?)\n```", block, re.DOTALL)
    require(match is not None, "recommended consumer workflow must be a yaml fenced block")
    return match.group(1) if match else ""


def assert_recommended_consumer_snippet(docs_text: str) -> None:
    snippet = recommended_snippet(docs_text)
    required = (
        "pull_request:",
        "types: [opened, ready_for_review, synchronize, reopened, unlabeled]",
        "permissions: {}",
        "github.event.pull_request.draft == false",
        "!contains(github.event.pull_request.labels.*.name, 'grimoire:disabled')",
        "github.event.label.name == '📋 Spec Needed'",
        "uses: DongwonTTuna-Labs/home-server-infra/.github/workflows/grimoire-control-plane.yml@main",
        "consumer_repository: ${{ github.repository }}",
        "consumer_ref: ${{ github.event.pull_request.head.ref }}",
        "pull_request_number: ${{ github.event.pull_request.number }}",
        "head_sha: ${{ github.event.pull_request.head.sha }}",
        "base_ref: ${{ github.event.pull_request.base.ref }}",
        "grimoire_app_client_id: Iv23liFL1dDHmU06FLSF",
        "GRIMOIRE_APP_PRIVATE_KEY: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}",
        "AI_RELAY_API_KEY: ${{ secrets.AI_RELAY_API_KEY }}",
        "CF_ACCESS_CLIENT_ID: ${{ secrets.CF_ACCESS_CLIENT_ID }}",
        "CF_ACCESS_CLIENT_SECRET: ${{ secrets.CF_ACCESS_CLIENT_SECRET }}",
    )
    for phrase in required:
        require(phrase in snippet, f"recommended consumer snippet missing: {phrase}")
    forbidden = ("secrets: inherit", "pull_request_target", "workflow_dispatch", "GITHUB_TOKEN", "GRIMOIRE_PAT", "ubuntu-latest")
    for phrase in forbidden:
        require(phrase not in snippet, f"recommended consumer snippet contains invalid pattern: {phrase}")
    print("consumer snippet ok")


def assert_invalid_examples(docs_text: str) -> None:
    invalid_markers = (
        "Invalid example, do not copy, SHA ref",
        "Invalid example, do not copy, tag ref",
        "Invalid example, do not copy, inherited secrets",
        "Invalid example, do not copy, GitHub-hosted fallback",
        "Invalid example, do not copy, legacy PAT or `GITHUB_TOKEN` auth",
        "Invalid example, do not copy, `pull_request_target`",
        "Invalid example, do not copy, runtime controls",
    )
    for marker in invalid_markers:
        require(marker in docs_text, f"missing invalid example marker: {marker}")
    print("invalid examples ok")


def assert_invalid_auth_guidance(docs_text: str, combined_text: str) -> None:
    required = (
        "PATs, `GITHUB_TOKEN`, `CODEX_LOOP_PAT`, `github.token`, and `secrets: inherit` are invalid for privileged Grimoire operations",
        "These forbidden legacy or caller-scoped credentials are not valid for privileged Grimoire GitHub operations: PAT, `GITHUB_TOKEN`, `CODEX_LOOP_PAT`, or `github.token`",
        "Grimoire must not recommend `pull_request_target` or GitHub-hosted runner fallback for privileged control-plane work",
        "GitHub-hosted runner fallback",
        "Invalid example, do not copy, legacy PAT or `GITHUB_TOKEN` auth",
        "GRIMOIRE_PAT: ${{ secrets.GITHUB_TOKEN }}",
    )
    for phrase in required:
        require(phrase in f"{combined_text}\n{docs_text}", f"invalid auth guidance missing: {phrase}")
    print("invalid auth guidance ok")


def assert_app_auth_adr(adr_text: str, docs_path: Path, docs_text: str) -> None:
    app_auth_adr_path = docs_path.parents[0] / "decisions" / "grimoire-app-auth.md"
    require(APP_AUTH_ADR_PATH in adr_text, "primary ADR must link the App-auth ADR path")
    require(APP_AUTH_ADR_PATH in docs_text, "docs must link the App-auth ADR path")
    app_auth_adr_text = read_text(app_auth_adr_path)
    assert_headings(app_auth_adr_text, APP_AUTH_ADR_HEADINGS, "App-auth ADR")
    for phrase in APP_AUTH_ADR_REQUIRED_PHRASES:
        require(phrase in app_auth_adr_text, f"App-auth ADR missing required phrase: {phrase}")
    print("App-auth ADR ok")


def assert_advisory_adr(docs_path: Path, docs_text: str) -> None:
    advisory_adr_path = docs_path.parents[0] / "decisions" / "grimoire-advisory-failure-separation.md"
    require(ADVISORY_ADR_PATH in docs_text, "docs must link the advisory/failure ADR path")
    advisory_adr_text = read_text(advisory_adr_path)
    assert_headings(advisory_adr_text, ADVISORY_ADR_HEADINGS, "advisory ADR")
    for phrase in ADVISORY_ADR_REQUIRED_PHRASES:
        require(phrase in advisory_adr_text, f"advisory ADR missing required phrase: {phrase}")
    print("advisory ADR ok")


def assert_release_notes(docs_path: Path, docs_text: str) -> None:
    release_path = docs_path.parents[0] / "releases" / "grimoire-reusable-control-plane-v1.md"
    advisory_release_path = docs_path.parents[0] / "releases" / "grimoire-reusable-control-plane-v1-advisory-failure.md"
    require(RELEASE_NOTES_PATH in docs_text, "docs must link the release note path")
    require(ADVISORY_RELEASE_NOTES_PATH in docs_text, "docs must link the advisory/failure release note path")
    release_text = read_text(release_path)
    advisory_release_text = read_text(advisory_release_path)
    for phrase in RELEASE_NOTES_REQUIRED_PHRASES:
        require(phrase in release_text, f"release notes missing required phrase: {phrase}")
    for phrase in ADVISORY_RELEASE_NOTES_REQUIRED_PHRASES:
        require(phrase in advisory_release_text, f"advisory release notes missing required phrase: {phrase}")
    print("release notes ok")


def assert_app_auth_release_notes(docs_path: Path, docs_text: str) -> None:
    release_path = docs_path.parents[0] / "releases" / "grimoire-reusable-control-plane-v1.md"
    app_auth_release_path = docs_path.parents[0] / "releases" / "grimoire-app-auth-v1.md"
    require(APP_AUTH_RELEASE_NOTES_PATH in docs_text, "docs must link the App-auth release note path")
    release_text = read_text(release_path)
    require(APP_AUTH_RELEASE_NOTES_PATH in release_text, "v1 release notes must link the App-auth release note path")
    app_auth_release_text = read_text(app_auth_release_path)
    assert_headings(app_auth_release_text, APP_AUTH_RELEASE_NOTES_HEADINGS, "App-auth release notes")
    for phrase in APP_AUTH_RELEASE_NOTES_REQUIRED_PHRASES:
        require(phrase in app_auth_release_text, f"App-auth release notes missing required phrase: {phrase}")
    print("App-auth release notes ok")


def assert_rollout_limitations(adr_text: str, docs_path: Path, docs_text: str) -> None:
    related_paths = (
        docs_path.parents[0] / "decisions" / "grimoire-app-auth.md",
        docs_path.parents[0] / "decisions" / "grimoire-advisory-failure-separation.md",
        docs_path.parents[0] / "releases" / "grimoire-reusable-control-plane-v1.md",
        docs_path.parents[0] / "releases" / "grimoire-reusable-control-plane-v1-advisory-failure.md",
        docs_path.parents[0] / "releases" / "grimoire-app-auth-v1.md",
    )
    related_text = "\n".join(read_text(path) for path in related_paths)
    combined_text = f"{adr_text}\n{docs_text}\n{related_text}"
    required = (
        "doesn't claim production rollout, org-wide secret rollout, or observed cross-repo PR-event execution",
        "This package doesn't claim production rollout, live Grimoire capability, or cross-repo execution evidence.",
        "No production or live rollout claim.",
        "No production rollout claim.",
        "No observed cross-repo PR-event execution claim.",
        "No claim that organization-level secret or variable rollout is complete.",
    )
    for phrase in required:
        require(phrase in combined_text, f"rollout limitation missing: {phrase}")
    forbidden_patterns = (
        r"\bproduction rollout (?:is|was|has been) (?:complete|completed|done|enabled|available|successful)",
        r"\blive Grimoire capability (?:is|was|has been) (?:complete|available|enabled|verified|observed)",
        r"\borg-wide secret rollout (?:is|was|has been) (?:complete|completed|done|enabled|available)",
        r"\bobserved cross-repo PR-event execution (?:is|was|has been) (?:complete|completed|done|verified)",
    )
    for pattern in forbidden_patterns:
        require(re.search(pattern, combined_text, re.IGNORECASE) is None, f"docs contain rollout overclaim: {pattern}")
    print("rollout limitations ok")


def run_contract(adr_path: Path, docs_path: Path) -> None:
    adr_text = read_text(adr_path)
    docs_text = read_text(docs_path)
    combined_text = f"{adr_text}\n{docs_text}"

    assert_headings(adr_text, ADR_HEADINGS, "ADR")
    assert_headings(docs_text, DOC_HEADINGS, "docs")
    assert_required_phrases(combined_text)
    assert_stage_map(combined_text)
    assert_path_map(combined_text)
    assert_main_consumer_policy(combined_text)
    assert_scope_guard(combined_text)
    assert_recommended_consumer_snippet(docs_text)
    assert_invalid_examples(docs_text)
    assert_invalid_auth_guidance(docs_text, combined_text)
    assert_app_auth_adr(adr_text, docs_path, docs_text)
    assert_advisory_adr(docs_path, docs_text)
    assert_release_notes(docs_path, docs_text)
    assert_app_auth_release_notes(docs_path, docs_text)
    assert_rollout_limitations(adr_text, docs_path, docs_text)
    print("grimoire docs contract ok")


def parse_args(argv: list[str]) -> tuple[Path, Path]:
    parser = argparse.ArgumentParser(description="Validate Grimoire reusable docs contract.")
    _ = parser.add_argument("--adr", required=True, type=Path)
    _ = parser.add_argument("--docs", required=True, type=Path)
    namespace = parser.parse_args(argv)
    return cast(Path, namespace.adr), cast(Path, namespace.docs)


def main(argv: list[str]) -> int:
    adr_path, docs_path = parse_args(argv)
    try:
        run_contract(adr_path, docs_path)
    except ContractError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    return 0


def test_grimoire_docs_contract() -> None:
    root = Path(__file__).resolve().parents[1]
    run_contract(
        root / "docs" / "decisions" / "grimoire-reusable-control-plane.md",
        root / "docs" / "grimoire-reusable.md",
    )


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
