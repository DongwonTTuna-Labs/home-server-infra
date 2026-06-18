# pyright: reportAny=false, reportUnusedCallResult=false
from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Callable


EXPECTED_INPUTS = {
    "consumer_repository",
    "consumer_ref",
    "pull_request_number",
    "head_sha",
    "base_ref",
    "grimoire_contract_version",
    "grimoire_app_client_id",
}
EXPECTED_SECRETS = {
    "GRIMOIRE_APP_PRIVATE_KEY": True,
    "AI_RELAY_API_KEY": False,
    "CF_ACCESS_CLIENT_ID": False,
    "CF_ACCESS_CLIENT_SECRET": False,
}
APP_TOKEN_ACTION = "actions/create-github-app-token@fee1f7d63c2ff003460e3d139729b119787bc349"
APP_TOKEN_EXPR = "${{ steps.grimoire-app-token.outputs.token }}"
EXPECTED_CF_HEADERS = {
    "CF-Access-Client-Id": "{env:CF_ACCESS_CLIENT_ID}",
    "CF-Access-Client-Secret": "{env:CF_ACCESS_CLIENT_SECRET}",
}
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
FORBIDDEN_RUNTIME_INPUTS = {
    "mode",
    "dry_run",
    "dry-run",
    "allow_live",
    "allow-live",
    "simulate",
    "simulation",
}
TEMP_ROOT = Path("/var/folders/vz/hx33c759727ftq88cxbgp8r40000gn/T/opencode")


class ContractError(AssertionError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError as exc:
        raise ContractError(f"missing workflow: {path}") from exc


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
    keys: set[str] = set()
    pattern = re.compile(rf"^ {{{indent}}}([A-Za-z0-9_-]+)\s*:", re.MULTILINE)
    for match in pattern.finditer(text):
        keys.add(match.group(1))
    return keys


def block_scalar_bodies(text: str, key: str = "run") -> list[list[str]]:
    lines = text.splitlines()
    bodies: list[list[str]] = []
    for index, line in enumerate(lines):
        if not re.match(rf"^\s*{re.escape(key)}\s*:\s*\|", line):
            continue
        base_indent = indent_of(line)
        body: list[str] = []
        for child in lines[index + 1 :]:
            if child.strip() and indent_of(child) <= base_indent:
                break
            body.append(child)
        bodies.append(body)
    return bodies


def step_block(text: str, name: str) -> str:
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
    raise ContractError(f"missing workflow step: {name}")


def action_step_block(text: str, step_id: str) -> str:
    marker = f"    - id: {step_id}\n"
    start = text.find(marker)
    require(start != -1, f"missing action step: {step_id}")
    next_start = text.find("\n    - id:", start + len(marker))
    return text[start:] if next_start == -1 else text[start:next_start]


def assert_events(text: str) -> None:
    on_block = section_body(text, "on", 0)
    events = keys_at_indent(on_block, 2)
    require(events == {"workflow_call"}, f"workflow must expose only workflow_call, got {sorted(events)}")


def assert_permissions(text: str) -> None:
    require(re.search(r"(?m)^permissions\s*:\s*\{\}\s*$", text) is not None, "workflow must keep top-level permissions: {}")
    require(
        re.search(r"(?m)^    permissions\s*:\s*$\n^      contents\s*:\s*read\s*$", text) is not None,
        "workflow job must declare explicit contents: read permissions",
    )


def assert_inputs_and_secrets(text: str) -> None:
    inputs_block = section_body(text, "inputs", 4)
    input_keys = keys_at_indent(inputs_block, 6)
    require(input_keys == EXPECTED_INPUTS, f"workflow_call inputs drifted: {sorted(input_keys)}")
    forbidden = sorted(input_keys & FORBIDDEN_RUNTIME_INPUTS)
    require(not forbidden, "workflow_call must not expose runtime toggles: " + ", ".join(forbidden))

    secrets_block = section_body(text, "secrets", 4)
    declared = keys_at_indent(secrets_block, 6)
    require(declared == set(EXPECTED_SECRETS), f"workflow_call secrets drifted: {sorted(declared)}")
    for secret, required in EXPECTED_SECRETS.items():
        pattern = rf"(?ms)^      {re.escape(secret)}\s*:\s*$.*?^        required\s*:\s*{str(required).lower()}\s*$"
        require(re.search(pattern, secrets_block) is not None, f"{secret} must declare required: {str(required).lower()}")
    referenced = set(re.findall(r"secrets\.([A-Za-z_][A-Za-z0-9_]*)", text))
    undeclared = referenced - declared
    require(not undeclared, "workflow references undeclared secrets: " + ", ".join(sorted(undeclared)))
    require(not re.search(r"(?m)^\s*secrets\s*:\s*inherit\s*$", text), "secrets: inherit is forbidden")


def assert_runner_and_checkouts(text: str) -> None:
    require("group: Home Server Runners" in text, "workflow must target the Home Server Runners runner group")
    require("labels: dongwontuna-labs-runner" in text, "workflow must target the dongwontuna-labs-runner label")
    require(not re.search(r"\b(?:ubuntu|macos|windows)-latest\b", text), "GitHub-hosted runner fallback is forbidden")

    require(text.count("uses: actions/checkout@v6") == 2, "workflow must have exactly two explicit checkout steps")
    control = step_block(text, "Checkout trusted control plane")
    for snippet in (
        "uses: actions/checkout@v6",
        "repository: DongwonTTuna-Labs/home-server-infra",
        "ref: main",
        "token: ${{ steps.grimoire-app-token.outputs.token }}",
        "path: control-plane",
        "persist-credentials: false",
    ):
        require(snippet in control, f"control-plane checkout missing {snippet}")
    consumer = step_block(text, "Checkout consumer repository as data")
    for snippet in (
        "uses: actions/checkout@v6",
        "repository: ${{ inputs.consumer_repository }}",
        "ref: ${{ inputs.head_sha }}",
        "token: ${{ steps.grimoire-app-token.outputs.token }}",
        "path: consumer",
        "fetch-depth: 0",
        "persist-credentials: false",
    ):
        require(snippet in consumer, f"consumer data checkout missing {snippet}")
    require(text.count("persist-credentials: false") == 2, "both checkouts must disable persisted credentials")


def assert_stage_paths(text: str, repo_root: Path) -> None:
    for stage in EXPECTED_STAGES:
        action_path = repo_root / "actions" / "grimoire" / stage / "action.yml"
        require(action_path.is_file(), f"missing stage action: actions/grimoire/{stage}/action.yml")
    require(text.count("./control-plane/actions/grimoire/trusted-controller") == 1, "trusted-controller must be called exactly once")
    require(text.count("./control-plane/actions/grimoire/cast") == 1, "cast driver must be called exactly once")
    require("./actions/grimoire" not in text, "bare ./actions/grimoire paths are forbidden")


def assert_cast_action_label_and_output_wiring(repo_root: Path) -> None:
    cast_text = read_text(repo_root / "actions" / "grimoire" / "cast" / "action.yml")
    require(cast_text.count("id: labels-spec-needed") == 1, "cast action must require exactly one labels-spec-needed step")
    for output in ("conclusion", "summary"):
        require(f"  {output}:" in cast_text and f"value: ${{{{ steps.complete.outputs.{output} }}}}" in cast_text, f"cast action must expose {output} from complete")
    trusted_label_inputs = (
        "remote-apply: ${{ inputs.github-mutation-allowed == 'true' && env.GRIMOIRE_GITHUB_PAT != '' }}",
        "token: ${{ inputs.github-mutation-allowed == 'true' && env.GRIMOIRE_GITHUB_PAT || '' }}",
        "github-api-url: ${{ github.api_url }}",
    )
    for step_id in ("labels-running", "labels-done", "labels-fizzled", "labels-spec-needed"):
        block = action_step_block(cast_text, step_id)
        require("uses: ./control-plane/actions/grimoire/labels" in block, f"{step_id} must use the trusted labels action")
        for snippet in trusted_label_inputs:
            require(snippet in block, f"{step_id} must use trusted PAT label input {snippet}")
    spec_needed = action_step_block(cast_text, "labels-spec-needed")
    require("if: ${{ steps.decide.outputs.label_transition == 'spec-needed' }}" in spec_needed, "labels-spec-needed must be gated on spec-needed transition")
    require("transition: spec-needed" in spec_needed, "labels-spec-needed must pass spec-needed transition")


def assert_opencode_runtime_provisioning(text: str, repo_root: Path) -> None:
    setup = step_block(text, "Provision opencode runtime")
    setup_index = text.find("name: Provision opencode runtime")
    trusted_checkout_index = text.find("name: Checkout trusted control plane")
    consumer_checkout_index = text.find("name: Checkout consumer repository as data")
    preflight_index = text.find("name: Validate opencode runtime")
    require(trusted_checkout_index < setup_index < consumer_checkout_index, "opencode runtime provisioning must use the trusted control-plane checkout before consumer PR-head data checkout")
    require(setup_index < preflight_index, "opencode runtime provisioning must run before opencode validation")
    required_snippets = (
        "id: setup-opencode",
        "python3 control-plane/actions/grimoire/cast/scripts/setup_opencode.py",
    )
    for snippet in required_snippets:
        require(snippet in setup, f"opencode runtime provisioning missing {snippet}")
    forbidden_snippets = ("secrets.", "steps.auth.outputs", "GRIMOIRE_PAT", "AI_RELAY_API_KEY", "CF_ACCESS_CLIENT_ID", "CF_ACCESS_CLIENT_SECRET", "pull_request_target", "secrets: inherit", "ubuntu-latest")
    for snippet in forbidden_snippets:
        require(snippet not in setup, f"opencode runtime provisioning contains forbidden snippet {snippet}")
    setup_helper = repo_root / "actions" / "grimoire" / "cast" / "scripts" / "setup_opencode.py"
    require(setup_helper.is_file(), "opencode runtime provisioning helper is missing")
    require(setup_helper.stat().st_mode & 0o111 != 0, "opencode runtime provisioning helper must be executable")


def assert_opencode_runtime_preflight(text: str) -> None:
    preflight = step_block(text, "Validate opencode runtime")
    require(text.find("name: Validate opencode runtime") < text.find("name: Run cast driver"), "opencode runtime validation must run before cast driver")
    required_snippets = (
        "id: opencode",
        "command -v opencode",
        "--version",
        "missing-runtime:opencode-unavailable",
        "runtime-failed:opencode-command-failed",
    )
    for snippet in required_snippets:
        require(snippet in preflight, f"opencode runtime validation missing {snippet}")
    forbidden_snippets = ("GITHUB_PATH", "pull_request_target", "secrets: inherit", "ubuntu-latest")
    for snippet in forbidden_snippets:
        require(snippet not in preflight, f"opencode runtime validation contains forbidden snippet {snippet}")


def assert_auth_and_inline_shell(text: str) -> None:
    forbidden_auth = ("GITHUB_TOKEN", "github.token", "GRIMOIRE_PAT", "CODEX_LOOP_PAT", "steps.auth.outputs.github_pat", "app-id:")
    for marker in forbidden_auth:
        require(marker not in text, f"forbidden privileged auth marker: {marker}")

    token_step = step_block(text, "Mint Grimoire GitHub App installation token")
    for marker in (
        "id: grimoire-app-token",
        f"uses: {APP_TOKEN_ACTION}",
        "client-id: ${{ inputs.grimoire_app_client_id }}",
        "private-key: ${{ secrets.GRIMOIRE_APP_PRIVATE_KEY }}",
        "owner: DongwonTTuna-Labs",
    ):
        require(marker in token_step, f"GitHub App token step missing marker: {marker}")
    require(re.search(r"actions/create-github-app-token@[0-9A-Fa-f]{40}", token_step) is not None, "GitHub App token action must be SHA-pinned")
    require(f"GRIMOIRE_GITHUB_PAT: {APP_TOKEN_EXPR}" in text, "cast driver must receive the minted App token as downstream auth")
    require(".omo/evidence" not in text, ".omo/evidence must not be a runtime workflow coupling")
    required_auth_markers = (
        "GRIMOIRE_CF_ACCESS_CLIENT_ID_SECRET: ${{ secrets.CF_ACCESS_CLIENT_ID }}",
        "GRIMOIRE_CF_ACCESS_CLIENT_SECRET_SECRET: ${{ secrets.CF_ACCESS_CLIENT_SECRET }}",
        "resolve_required cf_access_client_id",
        "resolve_required cf_access_client_secret",
        "CF_ACCESS_CLIENT_ID: ${{ steps.auth.outputs.cf_access_client_id }}",
        "CF_ACCESS_CLIENT_SECRET: ${{ steps.auth.outputs.cf_access_client_secret }}",
    )
    for marker in required_auth_markers:
        require(marker in text, f"workflow missing CF Access auth marker: {marker}")

    for body in block_scalar_bodies(text):
        nonempty = [line for line in body if line.strip()]
        require(len(nonempty) <= 35, "workflow contains a large inline shell block instead of action-local helpers")
        body_text = "\n".join(body)
        require(not re.search(r"\b(?:python3?|node|ruby)\s+[-<]", body_text), "workflow must not embed interpreter heredocs or scripts")


def assert_opencode_provider_headers(repo_root: Path) -> None:
    config_path = repo_root / "config" / "grimoire" / "opencode.json"
    try:
        payload = json.loads(read_text(config_path))
    except json.JSONDecodeError as exc:
        raise ContractError(f"invalid OpenCode config JSON: {config_path}: {exc}") from exc
    forbidden_metadata = {
        "grimoire_policy",
        "controller_owned",
        "consumer_pr_head_config_trusted",
        "runtime_simulation_inputs_allowed",
        "credential_source",
        "privileged_github_auth",
    }
    present_metadata = sorted(forbidden_metadata & set(payload))
    require(not present_metadata, "OpenCode runtime config must not contain controller policy metadata: " + ", ".join(present_metadata))
    allowed_runtime_keys = {"$schema", "model", "small_model", "share", "autoupdate", "default_agent", "plugin", "provider", "agent"}
    extra_keys = sorted(set(payload) - allowed_runtime_keys)
    require(not extra_keys, "OpenCode runtime config contains schema-unsafe top-level keys: " + ", ".join(extra_keys))
    try:
        options = payload["provider"]["ai-relay"]["options"]
    except (KeyError, TypeError) as exc:
        raise ContractError("OpenCode AI relay provider options are missing") from exc
    require(options.get("apiKey") == "{env:AI_RELAY_API_KEY}", "OpenCode AI relay apiKey must remain env-backed")
    require(options.get("headers") == EXPECTED_CF_HEADERS, "OpenCode AI relay must inject env-backed Cloudflare Access headers")


def assert_workflow_contract(workflow_path: Path, repo_root: Path) -> None:
    text = read_text(workflow_path)
    assert_events(text)
    assert_permissions(text)
    assert_inputs_and_secrets(text)
    assert_runner_and_checkouts(text)
    assert_stage_paths(text, repo_root)
    assert_cast_action_label_and_output_wiring(repo_root)
    assert_opencode_runtime_provisioning(text, repo_root)
    assert_opencode_runtime_preflight(text)
    assert_auth_and_inline_shell(text)
    assert_opencode_provider_headers(repo_root)


def replace_once(text: str, old: str, new: str) -> str:
    require(old in text, f"negative fixture source snippet not found: {old[:80]}")
    return text.replace(old, new, 1)


def insert_before_permissions(text: str, insertion: str) -> str:
    return replace_once(text, "\npermissions: {}\n", f"\n{insertion}\npermissions: {{}}\n")


def large_inline_shell(text: str) -> str:
    lines = "\n".join(f"          echo contract-line-{index}" for index in range(40))
    return text + "\n      - name: Large inline workflow shell fixture\n        shell: bash\n        run: |\n" + lines + "\n"


def remove_step(text: str, name: str) -> str:
    return replace_once(text, step_block(text, name), "")


def run_cast_workflow_validator(repo_root: Path, workflow_path: Path) -> subprocess.CompletedProcess[str]:
    script = repo_root / "actions" / "grimoire" / "cast" / "scripts" / "cast_driver.py"
    return subprocess.run(["python3", str(script), "validate-workflow", "--workflow", str(workflow_path)], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False)


def assert_cast_workflow_validator_contract(workflow_path: Path, repo_root: Path) -> None:
    valid = run_cast_workflow_validator(repo_root, workflow_path)
    require(valid.returncode == 0, f"cast validator rejected valid workflow: stdout={valid.stdout} stderr={valid.stderr}")

    source = read_text(workflow_path)
    target_dir = Path(tempfile.mkdtemp(prefix="grimoire-cast-validator-negative-", dir=str(TEMP_ROOT)))
    cases: list[tuple[str, Callable[[str], str], str]] = [
        ("pat-auth", lambda text: replace_once(text, f"GRIMOIRE_GITHUB_PAT: {APP_TOKEN_EXPR}", "GRIMOIRE_GITHUB_PAT: ${{ secrets.GRIMOIRE_PAT }}"), "GRIMOIRE_PAT"),
        ("github-token-auth", lambda text: replace_once(text, f"token: {APP_TOKEN_EXPR}", "token: ${{ secrets.GITHUB_TOKEN }}"), "GITHUB_TOKEN"),
        ("secrets-inherit", lambda text: replace_once(text, "    steps:\n", "    secrets: inherit\n    steps:\n"), "secrets: inherit"),
        ("pull-request-target", lambda text: insert_before_permissions(text, "  pull_request_target:"), "pull_request_target"),
        (
            "github-hosted-runner",
            lambda text: replace_once(text, "    runs-on:\n      group: Home Server Runners\n      labels: dongwontuna-labs-runner", "    runs-on: ubuntu-latest"),
            "GitHub-hosted runner",
        ),
        ("unpinned-token-action", lambda text: replace_once(text, APP_TOKEN_ACTION, "actions/create-github-app-token@v3"), "40-character commit SHA"),
        ("missing-opencode-runtime-provisioning", lambda text: remove_step(text, "Provision opencode runtime"), "opencode runtime provisioning"),
        ("missing-opencode-runtime-preflight", lambda text: remove_step(text, "Validate opencode runtime"), "opencode runtime preflight"),
    ]
    for name, mutate, expected in cases:
        target = target_dir / f"{name}.yml"
        target.write_text(mutate(source), encoding="utf-8")
        invalid = run_cast_workflow_validator(repo_root, target)
        require(invalid.returncode != 0, f"cast validator accepted negative workflow: {name}")
        require(expected in invalid.stderr, f"cast validator missing clear {name} error: {invalid.stderr}")


def make_missing_stage_root() -> Path:
    root = Path(tempfile.mkdtemp(prefix="grimoire-missing-stage-", dir=str(TEMP_ROOT)))
    (root / ".github" / "workflows").mkdir(parents=True, exist_ok=True)
    return root


def assert_negative_fixtures(workflow_path: Path, repo_root: Path) -> None:
    TEMP_ROOT.mkdir(parents=True, exist_ok=True)
    source = read_text(workflow_path)
    target_dir = Path(tempfile.mkdtemp(prefix="grimoire-workflow-negative-", dir=str(TEMP_ROOT)))
    cases: list[tuple[str, Callable[[str], str], Path]] = [
        ("non-workflow-call-event", lambda text: replace_once(text, "  workflow_call:", "  pull_request:"), repo_root),
        ("extra-pull-request-event", lambda text: insert_before_permissions(text, "  pull_request:"), repo_root),
        ("pull-request-target", lambda text: insert_before_permissions(text, "  pull_request_target:"), repo_root),
        ("workflow-dispatch", lambda text: insert_before_permissions(text, "  workflow_dispatch:"), repo_root),
        ("push", lambda text: insert_before_permissions(text, "  push:"), repo_root),
        ("secrets-inherit", lambda text: replace_once(text, "    steps:\n", "    secrets: inherit\n    steps:\n"), repo_root),
        ("pat-auth", lambda text: replace_once(text, f"GRIMOIRE_GITHUB_PAT: {APP_TOKEN_EXPR}", "GRIMOIRE_GITHUB_PAT: ${{ secrets.GRIMOIRE_PAT }}"), repo_root),
        ("github-token-auth", lambda text: replace_once(text, f"token: {APP_TOKEN_EXPR}", "token: ${{ secrets.GITHUB_TOKEN }}"), repo_root),
        ("unpinned-token-action", lambda text: replace_once(text, APP_TOKEN_ACTION, "actions/create-github-app-token@v3"), repo_root),
        ("missing-top-permissions", lambda text: replace_once(text, "permissions: {}\n", ""), repo_root),
        (
            "github-hosted-runner",
            lambda text: replace_once(text, "    runs-on:\n      group: Home Server Runners\n      labels: dongwontuna-labs-runner", "    runs-on: ubuntu-latest"),
            repo_root,
        ),
        (
            "runtime-toggle-input",
            lambda text: replace_once(text, "      grimoire_contract_version:\n", "      mode:\n        description: Forbidden runtime toggle.\n        required: false\n        type: string\n      grimoire_contract_version:\n"),
            repo_root,
        ),
        ("missing-stage-action", lambda text: text, make_missing_stage_root()),
        ("large-inline-workflow-shell", large_inline_shell, repo_root),
        ("bare-local-action-path", lambda text: replace_once(text, "./control-plane/actions/grimoire/cast", "./actions/grimoire/cast"), repo_root),
        (
            "undeclared-secret",
            lambda text: replace_once(text, "          GRIMOIRE_RELAY_SECRET: ${{ secrets.AI_RELAY_API_KEY }}", "          GRIMOIRE_RELAY_SECRET: ${{ secrets.AI_RELAY_API_KEY }}\n          EXTRA_SECRET: ${{ secrets.EXTRA_SECRET }}"),
            repo_root,
        ),
        ("evidence-runtime-coupling", lambda text: replace_once(text, "mkdir -p consumer/.omo/ci", "mkdir -p consumer/.omo/ci\n          touch consumer/.omo/evidence/runtime.txt"), repo_root),
        ("missing-opencode-runtime-provisioning", lambda text: remove_step(text, "Provision opencode runtime"), repo_root),
        ("missing-opencode-runtime-preflight", lambda text: remove_step(text, "Validate opencode runtime"), repo_root),
    ]
    for name, mutate, case_root in cases:
        target = target_dir / f"{name}.yml"
        target.write_text(mutate(source), encoding="utf-8")
        try:
            assert_workflow_contract(target, case_root)
        except ContractError:
            continue
        raise ContractError(f"negative workflow fixture was accepted: {name}")


def repo_root_for(workflow_path: Path) -> Path:
    try:
        return workflow_path.resolve().parents[2]
    except IndexError as exc:
        raise ContractError(f"workflow must live under .github/workflows: {workflow_path}") from exc


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Validate Grimoire reusable workflow contract.")
    parser.add_argument("--workflow", required=True, type=Path)
    return parser.parse_args(argv)


def test_grimoire_workflow_contract() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    workflow_path = repo_root / ".github" / "workflows" / "grimoire-control-plane.yml"
    assert_workflow_contract(workflow_path, repo_root)
    assert_negative_fixtures(workflow_path, repo_root)
    assert_cast_workflow_validator_contract(workflow_path, repo_root)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    workflow_path = args.workflow.resolve()
    repo_root = repo_root_for(workflow_path)
    try:
        assert_workflow_contract(workflow_path, repo_root)
        assert_negative_fixtures(workflow_path, repo_root)
    except ContractError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    print("workflow contract ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
