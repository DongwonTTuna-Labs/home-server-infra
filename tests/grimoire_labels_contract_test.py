# pyright: reportAny=false, reportExplicitAny=false, reportUnknownMemberType=false, reportArgumentType=false, reportOptionalMemberAccess=false, reportUnusedCallResult=false, reportUnusedParameter=false
from __future__ import annotations

import argparse
import importlib.util
import json
import re
import urllib.error
import urllib.parse
from pathlib import Path
from typing import Any


class ContractError(AssertionError):
    pass


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ContractError(message)


def load_labels_module() -> Any:
    path = Path(__file__).resolve().parents[1] / "actions" / "grimoire" / "labels" / "scripts" / "labels.py"
    spec = importlib.util.spec_from_file_location("grimoire_labels", path)
    require(spec is not None and spec.loader is not None, f"unable to load labels helper: {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def labels_args(workspace: Path, transition: str, *, remote_apply: bool, token: str = "github_pat_fixturetokenfixturetoken123456", github_output: str = "") -> argparse.Namespace:
    return argparse.Namespace(
        transition=transition,
        consumer_workspace=str(workspace),
        state_file=".omo/ci/grimoire-label-state.txt",
        state_output="",
        status_output=".omo/ci/grimoire-label-status.json",
        repository="DongwonTTuna-Labs/rs-builder-relayer-client",
        pr_number="129",
        github_output=github_output,
        remote_apply="true" if remote_apply else "false",
        token=token,
        github_api_url="https://api.github.test",
    )


def write_state(workspace: Path, labels: list[str]) -> None:
    path = workspace / ".omo" / "ci" / "grimoire-label-state.txt"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("\n".join(labels) + ("\n" if labels else ""), encoding="utf-8")


def read_status(workspace: Path) -> dict[str, Any]:
    return json.loads((workspace / ".omo" / "ci" / "grimoire-label-status.json").read_text(encoding="utf-8"))


def decoded_label(path: str) -> str:
    return urllib.parse.unquote(path.rsplit("/", 1)[-1])


def test_done_transition_applies_managed_github_labels_when_remote_enabled(tmp_path: Path, monkeypatch: Any) -> None:
    module = load_labels_module()
    write_state(tmp_path, ["🔮 Casting…", "reviewer:human"])
    calls: list[dict[str, Any]] = []

    def fake_request(method: str, path: str, token: str, payload: dict[str, Any] | None, api_url: str) -> dict[str, Any]:
        calls.append({"method": method, "path": path, "payload": payload, "api_url": api_url, "token": token})
        return {}

    monkeypatch.setattr(module, "github_request", fake_request, raising=False)
    result = module.run(labels_args(tmp_path, "done", remote_apply=True))

    require(result == 0, "enabled done transition must succeed when all GitHub label calls succeed")
    require([call["method"] for call in calls] == ["DELETE", "DELETE", "POST"], "done transition must remove both non-Cast managed labels before adding Cast")
    require(decoded_label(calls[0]["path"]) == "🔮 Casting…", "done transition must remove the running managed label remotely")
    require(decoded_label(calls[1]["path"]) == "💨 Fizzled", "done transition must remove the fizzled managed label remotely even when local state lacks it")
    require(calls[2]["payload"] == {"labels": ["✨ Cast"]}, "done transition must add only the Cast managed label remotely")
    status = read_status(tmp_path)
    require(status["github_pr_label_mutation_attempted"] is True, "enabled remote apply must record github_pr_label_mutation_attempted=true")
    require(status["labels_are_display_only"] is False, "enabled remote apply must no longer report display-only label behavior")
    require(status["final_labels"] == ["reviewer:human", "✨ Cast"], "local final state must preserve unrelated labels and end at Cast")


def test_disabled_or_tokenless_mode_never_calls_github_and_records_not_attempted(tmp_path: Path, monkeypatch: Any) -> None:
    module = load_labels_module()
    calls: list[dict[str, Any]] = []

    def fake_request(method: str, path: str, token: str, payload: dict[str, Any] | None, api_url: str) -> dict[str, Any]:
        calls.append({"method": method, "path": path, "payload": payload, "api_url": api_url, "token": token})
        return {}

    monkeypatch.setattr(module, "github_request", fake_request, raising=False)
    disabled_workspace = tmp_path / "disabled"
    write_state(disabled_workspace, ["🔮 Casting…"])
    require(module.run(labels_args(disabled_workspace, "done", remote_apply=False)) == 0, "disabled remote apply should keep local-only behavior")
    disabled_status = read_status(disabled_workspace)
    require(disabled_status["github_pr_label_mutation_attempted"] is False, "disabled remote apply must record no GitHub mutation attempt")

    tokenless_workspace = tmp_path / "tokenless"
    write_state(tokenless_workspace, ["🔮 Casting…"])
    require(module.run(labels_args(tokenless_workspace, "done", remote_apply=True, token="")) == 0, "tokenless remote apply should not call GitHub")
    tokenless_status = read_status(tokenless_workspace)
    require(tokenless_status["github_pr_label_mutation_attempted"] is False, "tokenless remote apply must record no GitHub mutation attempt")
    require(calls == [], "disabled or tokenless label mode must not call GitHub")


def test_remote_remove_missing_managed_label_is_idempotent(tmp_path: Path, monkeypatch: Any) -> None:
    module = load_labels_module()
    write_state(tmp_path, ["🔮 Casting…"])
    calls: list[dict[str, Any]] = []

    def fake_request(method: str, path: str, token: str, payload: dict[str, Any] | None, api_url: str) -> dict[str, Any]:
        calls.append({"method": method, "path": path, "payload": payload, "api_url": api_url, "token": token})
        if method == "DELETE" and decoded_label(path) == "💨 Fizzled":
            raise urllib.error.HTTPError(api_url + path, 404, "Not Found", {}, None)
        return {}

    monkeypatch.setattr(module, "github_request", fake_request, raising=False)
    result = module.run(labels_args(tmp_path, "done", remote_apply=True))

    require(result == 0, "404 while removing an absent managed label must be treated idempotently")
    require([call["method"] for call in calls] == ["DELETE", "DELETE", "POST"], "idempotent missing removes must not skip later add operations")
    status = read_status(tmp_path)
    require(status["github_pr_label_mutation_attempted"] is True, "idempotent remove path still attempted GitHub mutation")
    require(any(result.get("status") == "missing" for result in status["github_label_results"]), "status artifact must record the idempotent missing-label remove")


def test_spec_needed_transition_removes_other_managed_labels_preserves_unrelated_and_is_idempotent(tmp_path: Path) -> None:
    module = load_labels_module()
    write_state(tmp_path, ["🔮 Casting…", "💨 Fizzled", "✨ Cast", "reviewer:human", "area:docs"])

    require(module.run(labels_args(tmp_path, "spec-needed", remote_apply=False)) == 0, "spec-needed transition must succeed locally")
    status = read_status(tmp_path)
    require(status["final_labels"] == ["reviewer:human", "area:docs", "📋 Spec Needed"], "spec-needed must remove running/done/fizzled managed labels and preserve unrelated labels")
    require([operation["action"] for operation in status["operations"]] == ["remove", "remove", "remove", "add"], "spec-needed must scope operations to the managed transition")
    require(status["unrelated_labels_preserved"] == ["area:docs", "reviewer:human"], "spec-needed must report preserved unrelated labels")

    require(module.run(labels_args(tmp_path, "spec-needed", remote_apply=False)) == 0, "second spec-needed transition must remain idempotent")
    repeated = read_status(tmp_path)
    require(repeated["final_labels"] == status["final_labels"], "idempotent spec-needed transition must not change final labels")
    require(repeated["operation_count"] == 0 and repeated["changed"] is False, "idempotent spec-needed transition must emit no local operations")



def test_remote_add_failure_fails_closed_with_sanitized_error(tmp_path: Path, monkeypatch: Any) -> None:
    module = load_labels_module()
    secret = "github_pat_secretsecretsecretsecret1234567890"
    write_state(tmp_path, ["🔮 Casting…"])

    def fake_request(method: str, path: str, token: str, payload: dict[str, Any] | None, api_url: str) -> dict[str, Any]:
        if method == "POST":
            raise urllib.error.HTTPError(api_url + path, 500, f"server rejected token {secret}", {}, None)
        return {}

    monkeypatch.setattr(module, "github_request", fake_request, raising=False)
    result = module.run(labels_args(tmp_path, "done", remote_apply=True, token=secret))

    require(result == 1, "remote add failure must fail closed")
    status = read_status(tmp_path)
    serialized = json.dumps(status, ensure_ascii=False, sort_keys=True)
    require(status["github_pr_label_mutation_attempted"] is True, "failed remote add still must record that GitHub mutation was attempted")
    require(status["remote_apply_status"] == "failed", "failed remote add must be explicit in the status artifact")
    require(secret not in serialized and "github_pat_" not in serialized, "status artifact must not leak PAT values or prefixes")
    require("[REDACTED]" in serialized, "sanitized failure should preserve a redaction marker for diagnostics")


def step_block(text: str, step_id: str) -> str:
    marker = f"    - id: {step_id}\n"
    start = text.find(marker)
    require(start != -1, f"missing cast step: {step_id}")
    next_start = text.find("\n    - id:", start + len(marker))
    return text[start:] if next_start == -1 else text[start:next_start]


def test_cast_action_wires_label_remote_apply_only_through_trusted_pat_path() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    labels_text = (repo_root / "actions" / "grimoire" / "labels" / "action.yml").read_text(encoding="utf-8")
    cast_text = (repo_root / "actions" / "grimoire" / "cast" / "action.yml").read_text(encoding="utf-8")

    for key in ("remote-apply", "token", "github-api-url"):
        require(f"  {key}:" in labels_text, f"labels action must expose explicit {key} input")
    require(cast_text.count("id: labels-spec-needed") == 1, "cast action must define exactly one labels-spec-needed step")
    for output in ("conclusion", "summary"):
        require(re.search(rf"(?m)^  {output}:\n(?:^    .+\n)+?^    value: \${{{{ steps\.complete\.outputs\.{output} }}}}", cast_text) is not None, f"cast action must wire {output} output from complete")
    trusted_label_inputs = (
        "remote-apply: ${{ inputs.github-mutation-allowed == 'true' && env.GRIMOIRE_GITHUB_PAT != '' }}",
        "token: ${{ inputs.github-mutation-allowed == 'true' && env.GRIMOIRE_GITHUB_PAT || '' }}",
        "github-api-url: ${{ github.api_url }}",
    )
    for step_id in ("labels-running", "labels-done", "labels-fizzled", "labels-spec-needed"):
        block = step_block(cast_text, step_id)
        for snippet in trusted_label_inputs:
            require(snippet in block, f"{step_id} must pass trusted label input {snippet}")
    spec_needed = step_block(cast_text, "labels-spec-needed")
    require("if: ${{ steps.decide.outputs.label_transition == 'spec-needed' }}" in spec_needed, "labels-spec-needed must be gated only by the spec-needed transition")
    require("transition: spec-needed" in spec_needed, "labels-spec-needed must pass the spec-needed transition")
    for forbidden in ("GITHUB_TOKEN", "github.token", "pull_request_target", "secrets: inherit"):
        require(forbidden not in labels_text + cast_text, f"label/cast actions must not introduce forbidden auth or trigger pattern: {forbidden}")
