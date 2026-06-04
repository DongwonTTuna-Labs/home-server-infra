"""Permission-ceiling contract for the Codex loop reusable-core pipeline.

The reusable core keeps validation/topology/stage skeleton jobs read-only on the
repo, grants ``id-token: write`` only to the common relay setup job that must mint
an OIDC relay token, confines repo permission writes to ``finalize-stage``, and wires
GitHub App push tokens only inside the trusted ``fix-push`` mutation job.
"""
from _pipeline import (
    DISPATCH,
    MANUAL,
    REUSABLE,
    all_text,
    load,
    reusable_jobs,
)

PIPELINE_FILES = (REUSABLE, DISPATCH, MANUAL)
REUSABLE_PERMISSION_CEILING = {
    "contents": "write",
    "pull-requests": "write",
    "id-token": "write",
}
STAGE_JOBS = (
    "review-collect", "review-axes", "review-combine", "review-techlead",
    "design-prepare", "design-analyze", "design-plan", "design-chief",
    "fix-dispatch", "fix-run-agents", "fix-merge-validate", "fix-push", "issue-stage",
)


def test_all_workflows_declare_empty_top_level_permissions():
    for path in PIPELINE_FILES:
        text = path.read_text(encoding="utf-8")
        assert "permissions: {}" in text, path.name


def test_validation_and_stage_jobs_have_no_repo_write_permissions():
    for name, job in reusable_jobs().items():
        if name == "finalize-stage":
            continue
        perms = job.get("permissions", {})
        assert perms.get("contents") == "read", name
        assert perms.get("pull-requests") == "read", name
        assert perms.get("issues") != "write", name
        repo_scopes = {k: v for k, v in perms.items() if k != "id-token"}
        assert "write" not in set(repo_scopes.values()), name


def test_only_finalize_stage_holds_repo_permissions_and_fix_push_holds_app_token_path():
    jobs = reusable_jobs()
    finalize_perms = jobs["finalize-stage"].get("permissions", {})
    assert finalize_perms.get("contents") == "write"
    assert finalize_perms.get("pull-requests") == "write"
    write_jobs = [
        name
        for name, job in jobs.items()
        if "write" in set(job.get("permissions", {}).values())
        and any(v == "write" for k, v in job.get("permissions", {}).items() if k != "id-token")
    ]
    assert write_jobs == ["finalize-stage"], write_jobs
    assert "codex-review auth app-token" in str(jobs["fix-push"])
    assert "--mode push" in str(jobs["fix-push"])
    assert "codex-review auth app-token" in str(jobs["finalize-stage"])
    assert "--mode dispatch" in str(jobs["finalize-stage"])
    push_envs = [step.get("env", {}) for step in jobs["fix-push"].get("steps", [])]
    assert any(env.get("GITHUB_TOKEN") == "${{ steps.app_token.outputs.token }}" for env in push_envs)
    assert all(env.get("GITHUB_TOKEN") != "${{ github.token }}" for env in push_envs)
    dispatch_envs = [step.get("env", {}) for step in jobs["finalize-stage"].get("steps", [])]
    assert any(env.get("DISPATCH_APP_TOKEN") == "${{ steps.dispatch_app_token.outputs.token }}" for env in dispatch_envs)
    assert all(env.get("DISPATCH_APP_TOKEN") != "${{ github.token }}" for env in dispatch_envs)


def test_stage_skeleton_jobs_are_read_only_without_id_token():
    jobs = reusable_jobs()
    for name in STAGE_JOBS:
        perms = jobs[name].get("permissions", {})
        assert perms.get("contents") == "read", name
        assert perms.get("pull-requests") == "read", name
        assert perms.get("id-token") != "write", name
        repo_scopes = {k: v for k, v in perms.items() if k != "id-token"}
        assert "write" not in set(repo_scopes.values()), name


def test_relay_setup_uses_native_oidc_without_repo_write():
    jobs = reusable_jobs()
    perms = jobs["setup-relay"].get("permissions", {})
    assert perms.get("id-token") == "write"
    assert perms.get("contents") == "read"
    assert perms.get("pull-requests") == "read"
    assert "codex-review oidc relay-token" in str(jobs["setup-relay"])
    repo_scopes = {k: v for k, v in perms.items() if k != "id-token"}
    assert "write" not in set(repo_scopes.values())


def test_adapters_grant_documented_reusable_permission_ceiling():
    for path in (DISPATCH, MANUAL):
        text = path.read_text(encoding="utf-8")
        assert "uses: ./.github/workflows/codex-loop-reusable.yml" in text, path.name
        workflow = load(path)
        caller_jobs = [
            job
            for job in workflow["jobs"].values()
            if str(job.get("uses", "")).endswith("codex-loop-reusable.yml")
        ]
        assert caller_jobs, path.name
        for job in caller_jobs:
            perms = job.get("permissions", {})
            for scope, level in REUSABLE_PERMISSION_CEILING.items():
                assert perms.get(scope) == level, f"{path.name}:{scope}"


def test_no_workflow_uses_write_all_permissions():
    for path in PIPELINE_FILES:
        text = path.read_text(encoding="utf-8")
        assert "write-all" not in text, path.name


def test_no_pat_or_writable_default_github_token_is_wired():
    text = all_text()
    assert "PERSONAL_ACCESS_TOKEN" not in text
    assert "GH_PAT" not in text
    assert "GITHUB_TOKEN: ${{ github.token }}" not in text
    assert "codex-review auth app-token" in str(reusable_jobs()["fix-push"])
    assert "codex-review auth app-token" in str(reusable_jobs()["finalize-stage"])
    for name, job in reusable_jobs().items():
        if name not in {"fix-push", "finalize-stage"}:
            assert "auth app-token" not in str(job), name


def test_no_core_job_grants_issues_write():
    for name, job in reusable_jobs().items():
        perms = job.get("permissions", {})
        assert perms.get("issues") != "write", name
