"""Contract for the simplified Codex loop core.

Private-repo infra: no OIDC, no GitHub App, no trust/security gates, no loop cap.
The core is four jobs — validate -> setup-state -> run-stage -> finalize — that
run live via a static relay key and a permanent PAT, looping until LGTM.
"""
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
CORE = ROOT / ".github" / "workflows" / "codex-loop-reusable.yml"
RELAY_ENDPOINT = "https://relay-ai.dongwontuna.net/v1/responses"
MODEL_ACTION = "openai/codex-action@v1"


def _text() -> str:
    return CORE.read_text(encoding="utf-8")


def _doc() -> dict:
    return yaml.safe_load(_text())


def test_four_job_pipeline_in_order():
    jobs = list(_doc()["jobs"].keys())
    assert jobs == ["validate", "setup-state", "run-stage", "finalize"], jobs


def test_secrets_are_static_key_and_pat_only():
    on = _doc().get("on", _doc().get(True))
    secrets = on["workflow_call"]["secrets"]
    assert set(secrets) == {"CODEX_RELAY_API_KEY", "CODEX_LOOP_PAT"}, sorted(secrets)


def test_no_deleted_machinery_remains():
    text = _text()
    for token in (
        "oidc", "relay-token", "relay_configured", "live_ready",
        "trust-and-stale-guard", "eligible", "auth app-token", "app_token",
        "CODEX_GITHUB_APP", "max_iterations", "guard-dispatch",
        "append-dispatch-ledger", "id-token",
    ):
        assert token not in text, f"deleted machinery still present: {token}"


def test_model_steps_use_static_relay_key_and_endpoint():
    seen = 0
    for job in _doc()["jobs"].values():
        for step in job.get("steps", []) or []:
            if step.get("uses") == MODEL_ACTION:
                seen += 1
                w = step.get("with") or {}
                assert w.get("openai-api-key") == "${{ secrets.CODEX_RELAY_API_KEY }}"
                assert w.get("responses-api-endpoint") == RELAY_ENDPOINT
    assert seen >= 9, f"expected >=9 model steps, saw {seen}"


def test_push_and_dispatch_use_the_pat():
    text = _text()
    assert "PUSH_TOKEN: ${{ secrets.CODEX_LOOP_PAT }}" in text
    assert "GH_TOKEN: ${{ secrets.CODEX_LOOP_PAT }}" in text


def test_continuation_dispatch_gates_only_on_dispatch_candidate():
    dispatch_ifs = [
        step.get("if", "")
        for step in _doc()["jobs"]["finalize"]["steps"]
        if "dispatches" in str(step.get("run", "")) or "repository_dispatch" in str(step.get("name", "")).lower()
    ]
    assert dispatch_ifs, "expected a continuation dispatch step"
    for cond in dispatch_ifs:
        assert cond == "${{ steps.resolve.outputs.dispatch_candidate == 'true' }}", cond
