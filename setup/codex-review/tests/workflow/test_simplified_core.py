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


def test_no_workflow_call_secrets_and_no_secret_refs():
    # Credentials come from runner env vars, not GitHub secrets.
    on = _doc().get("on", _doc().get(True))
    assert "secrets" not in on["workflow_call"], "core must declare no workflow_call secrets"
    assert "${{ secrets." not in _text(), "no secrets.* references allowed"


def test_no_deleted_machinery_remains():
    text = _text()
    for token in (
        "oidc", "relay-token", "relay_configured", "live_ready",
        "trust-and-stale-guard", "eligible", "auth app-token", "app_token",
        "CODEX_GITHUB_APP", "max_iterations", "guard-dispatch",
        "append-dispatch-ledger", "id-token",
    ):
        assert token not in text, f"deleted machinery still present: {token}"


def test_relay_key_read_from_runner_env_and_fed_to_model():
    text = _text()
    # A capture step reads the runner env var (fail-fast) and exposes it.
    assert "${CODEX_RELAY_API_KEY:?" in text
    assert 'echo "::add-mask::${CODEX_RELAY_API_KEY}"' in text
    seen = 0
    structured = 0
    fix_agents_seen = False
    homes = []
    for job in _doc()["jobs"].values():
        for step in job.get("steps", []) or []:
            if step.get("uses") == MODEL_ACTION:
                seen += 1
                w = step.get("with") or {}
                assert w.get("openai-api-key") == "${{ steps.relay_key.outputs.key }}"
                assert w.get("responses-api-endpoint") == RELAY_ENDPOINT
                # Container is the isolation boundary; the action's sudo-drop
                # sandbox needs passwordless sudo the runner doesn't grant.
                assert w.get("safety-strategy") == "unsafe"
                # Runner container can't nest user namespaces, so codex must not
                # use bubblewrap — run commands directly (full access).
                assert w.get("sandbox") == "danger-full-access", step.get("name")
                assert w.get("working-directory") == "pr-head", step.get("name")
                # Fresh per-step Codex home (persistent runner would otherwise
                # accumulate duplicate keys in ~/.codex/config.toml).
                ch = w.get("codex-home", "")
                assert ch.startswith("${{ runner.temp }}/codex-home/"), step.get("name")
                homes.append(ch)
                name = step.get("name", "")
                if not name.startswith("Run live Codex"):
                    continue
                if "fix agents" in name:
                    # Multi-file emitter writes agents/*/result.json itself.
                    fix_agents_seen = True
                else:
                    # Single-JSON steps capture deterministic output via the
                    # codex-action structured-output contract.
                    assert w.get("output-file"), f"missing output-file: {name}"
                    assert w.get("output-schema-file"), (
                        f"missing output-schema-file: {name}"
                    )
                    structured += 1
    assert seen >= 9, f"expected >=9 model steps, saw {seen}"
    assert structured >= 8, f"expected >=8 structured-output steps, saw {structured}"
    assert fix_agents_seen, "fix agents step not found"
    # Per-step codex-home must be unique so invocations never share config.toml.
    assert len(homes) == len(set(homes)) == seen, homes
    # Each structured step emits its OpenAI strict schema before running.
    assert text.count("schema openai-strict") >= 8


def test_push_and_dispatch_use_the_pat_from_runner_env():
    text = _text()
    assert "${CODEX_LOOP_PAT:?" in text
    assert '--token "${CODEX_LOOP_PAT}"' in text
    assert 'export GH_TOKEN="${CODEX_LOOP_PAT}"' in text


def test_loop_scratch_lives_under_runner_temp():
    # Self-hosted runners reuse the workspace between jobs but wipe RUNNER_TEMP
    # per job. The stage-decision gate and the loop-state file are per-job
    # scratch; keeping them under RUNNER_TEMP makes cross-job leakage impossible
    # by construction. (A stale workspace-root gate previously made a later
    # stage short-circuit and re-emit the prior decision -> design->design loop.)
    # Mirrors the codex-home pattern already used by the model steps.
    text = _text()
    # The decision gate must never live at the workspace root.
    assert ".codex-loop-stage-result.json" not in text
    assert "${RUNNER_TEMP}/codex-loop-stage-result.json" in text
    # The loop-state file is never read/written/uploaded from the workspace root.
    for bare in (
        "--out codex-loop-state.json",
        "--loop-state codex-loop-state.json",
        "path: codex-loop-state.json",
    ):
        assert bare not in text, bare
    assert '--out "${RUNNER_TEMP}/codex-loop-state.json"' in text
    assert "${{ runner.temp }}/codex-loop-state.json" in text
    # The rm-based reset band-aid is replaced by the runner.temp guarantee.
    assert "Reset stale loop workspace scratch" not in text


def test_cross_run_artifact_downloads_pass_github_token():
    # download-artifact@v4 does NOT default github-token for cross-run
    # downloads (run-id set): without it the action only searches the current
    # run and the prior-stage state artifact is "not found". The token is the
    # job's own GITHUB_TOKEN, which has actions: read for same-repo runs.
    cross_run = []
    for job in _doc()["jobs"].values():
        for step in job.get("steps", []) or []:
            w = step.get("with") or {}
            if str(step.get("uses", "")).startswith("actions/download-artifact") and "run-id" in w:
                cross_run.append((step.get("name", ""), w))
    assert cross_run, "expected at least one cross-run artifact download"
    for name, w in cross_run:
        assert w.get("github-token") == "${{ github.token }}", name


def test_continuation_dispatch_gates_only_on_dispatch_candidate():
    dispatch_ifs = [
        step.get("if", "")
        for step in _doc()["jobs"]["finalize"]["steps"]
        if "dispatches" in str(step.get("run", "")) or "repository_dispatch" in str(step.get("name", "")).lower()
    ]
    assert dispatch_ifs, "expected a continuation dispatch step"
    for cond in dispatch_ifs:
        assert cond == "${{ steps.resolve.outputs.dispatch_candidate == 'true' }}", cond
