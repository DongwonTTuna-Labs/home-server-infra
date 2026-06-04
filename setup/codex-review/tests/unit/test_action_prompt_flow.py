import json
import subprocess

from codex_review.cli import main
from codex_review.stages.review.prompt import build_axis_prompt
from codex_review.stages.techlead.prompt import build_techlead_prompt
from codex_review.stages.design_chief.prompt import build_design_chief_prompt


def write_json(path, payload):
    path.write_text(json.dumps(payload), encoding="utf-8")


def assert_inspection_contract(prompt: str):
    assert "pr-head" in prompt
    assert "inspection_evidence" in prompt
    assert "path" in prompt
    assert "purpose" in prompt
    assert "observation" in prompt
    assert "existing file" in prompt
    assert "missing file" in prompt


def test_design_prompt_only_commands_write_model_prompts(tmp_path):
    context = tmp_path / "design-context.json"
    inventory = tmp_path / "design-inventory.json"
    clusters = tmp_path / "design-clusters.json"
    analysis = tmp_path / "cluster-analysis.json"
    write_json(
        context,
        {
            "schema_version": "design-context.v1",
            "findings": [{"finding_id": "f1", "summary": "needs design"}],
            "techlead_decision": {"decisions": [{"finding_id": "f1", "action": "needs_design"}]},
        },
    )
    write_json(inventory, {"schema_version": "design-inventory.v1", "items": [{"finding_id": "f1", "summary": "needs design"}]})
    write_json(clusters, {"schema_version": "design-clusters.v1", "clusters": [{"cluster_id": "c1", "finding_ids": ["f1"]}]})
    write_json(analysis, {"schema_version": "design-cluster-analysis.v1", "analyses": [{"cluster_id": "c1", "status": "ready"}]})

    cases = [
        (["design", "build-inventory-prompt", "--in", str(context)], "design-inventory.v1"),
        (["design", "build-clusters-prompt", "--inventory", str(inventory), "--pr-context", str(context)], "design-clusters.v1"),
        (["design", "build-analysis-prompt", "--inventory", str(clusters), "--pr-context", str(context)], "design-cluster-analysis.v1"),
        (["design", "build-plan-prompt", "--pr-context", str(context), "--inventory", str(clusters), "--result", str(analysis)], "design-plan.v1"),
    ]
    for idx, (args, expected) in enumerate(cases, 1):
        out = tmp_path / f"prompt-{idx}.md"
        assert main([*args, "--out", str(out)]) == 0
        assert expected in out.read_text(encoding="utf-8")


def test_design_plan_prompt_keeps_human_routing_in_design_chief(tmp_path):
    context = tmp_path / "design-context.json"
    clusters = tmp_path / "design-clusters.json"
    analysis = tmp_path / "cluster-analysis.json"
    write_json(context, {"schema_version": "design-context.v1", "findings": [{"finding_id": "f1"}]})
    write_json(clusters, {"schema_version": "design-clusters.v1", "clusters": [{"cluster_id": "c1"}]})
    write_json(analysis, {"schema_version": "design-cluster-analysis.v1", "analyses": [{"cluster_id": "c1"}]})

    out = tmp_path / "prompt.md"
    assert main([
        "design",
        "build-plan-prompt",
        "--pr-context",
        str(context),
        "--inventory",
        str(clusters),
        "--result",
        str(analysis),
        "--out",
        str(out),
    ]) == 0

    prompt = out.read_text(encoding="utf-8")
    assert "candidate design plan" in prompt
    assert "design_chief" in prompt
    assert "open_questions" not in prompt
    assert_inspection_contract(prompt)


def test_openspec_backed_prompts_drive_closed_implementation_plan(tmp_path):
    context = tmp_path / "design-context.json"
    clusters = tmp_path / "design-clusters.json"
    analysis = tmp_path / "cluster-analysis.json"
    write_json(
        context,
        {
            "schema_version": "design-context.v1",
            "openspec_backed": True,
            "openspec_context": {"source_summary": ["openspec/changes/demo/tasks.md"]},
            "findings": [{"finding_id": "f1"}],
        },
    )
    write_json(clusters, {"schema_version": "design-clusters.v1", "clusters": [{"cluster_id": "c1"}]})
    write_json(analysis, {"schema_version": "design-cluster-analysis.v1", "analyses": [{"cluster_id": "c1"}]})

    out = tmp_path / "prompt.md"
    assert main([
        "design",
        "build-plan-prompt",
        "--pr-context",
        str(context),
        "--inventory",
        str(clusters),
        "--result",
        str(analysis),
        "--out",
        str(out),
    ]) == 0

    prompt = out.read_text(encoding="utf-8")
    assert "OpenSpec-backed implementation" in prompt
    assert "acceptance_criteria" in prompt
    assert_inspection_contract(prompt)
    assert "questions" not in prompt.lower()


def test_review_techlead_design_chief_prompts_require_repo_inspection_evidence():
    pr_context = {"changed_line_map": {"src/lib.rs": [1]}, "title": "demo"}
    combined = {"findings": [{"finding_id": "f1", "file": "src/lib.rs", "line": 1}]}
    review = build_axis_prompt("correctness", pr_context, "", "docs", {"review": {"axes": ["correctness"]}})
    techlead = build_techlead_prompt(combined, pr_context, "", "docs", {})
    design_chief = build_design_chief_prompt(
        {"schema_version": "design-plan.v1", "edit_sequence": [{"task_id": "t1"}], "tests": ["cargo test"]},
        {"schema_version": "techlead-decision.v1", "decisions": [{"finding_id": "f1", "action": "needs_design"}]},
        pr_context,
        {"autofix": {"allowed_prefixes": ["src/"], "max_tasks": 1}},
    )
    for prompt in [review, techlead, design_chief]:
        assert_inspection_contract(prompt)


def test_fix_dispatch_prepare_agents_writes_prompts_matrix_and_github_outputs(tmp_path, monkeypatch):
    manifest = tmp_path / "manifest.json"
    design_plan = tmp_path / "design-plan.json"
    chief = tmp_path / "chief.json"
    docs = tmp_path / "docs.md"
    matrix = tmp_path / "matrix.json"
    gh_output = tmp_path / "github-output"
    work_dir = tmp_path / "agents"
    write_json(manifest, {"schema_version": "fix-dispatch-task-manifest.v1", "tasks": [{"task_id": "fix/one", "summary": "Fix it", "allowed_files": ["src/lib.rs"]}]})
    write_json(design_plan, {"schema_version": "design-plan.v1", "edit_sequence": [], "tests": ["cargo test"]})
    write_json(chief, {"schema_version": "design-chief-decision.v1", "status": "approved_for_fix", "fix_policy": {"allowed_files": ["src/lib.rs"]}})
    docs.write_text("docs", encoding="utf-8")
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))

    assert main([
        "fix_dispatch",
        "prepare-agents",
        "--inventory",
        str(manifest),
        "--design-plan",
        str(design_plan),
        "--chief-decision",
        str(chief),
        "--docs-context",
        str(docs),
        "--repo-path",
        "pr-head",
        "--work-dir",
        str(work_dir),
        "--out",
        str(matrix),
    ]) == 0

    payload = json.loads(matrix.read_text(encoding="utf-8"))
    assert payload["include"][0]["task_id"] == "fix/one"
    assert payload["include"][0]["task_path"] == "fix_one"
    prompt_path = work_dir / "fix_one" / "prompt.md"
    task_path = work_dir / "fix_one" / "task.json"
    assert prompt_path.is_file()
    assert task_path.is_file()
    assert "src/lib.rs" in prompt_path.read_text(encoding="utf-8")
    output_text = gh_output.read_text(encoding="utf-8")
    assert "has_agent_tasks=true" in output_text
    assert "agent_matrix=" in output_text


def test_fix_merge_prepare_merge_model_routes_without_invoking_model_when_clean(tmp_path, monkeypatch):
    premerge = tmp_path / "premerge.json"
    collection = tmp_path / "collection.json"
    pr_context = tmp_path / "pr-context.json"
    raw_out = tmp_path / "merged-fix.raw.json"
    route = tmp_path / "merge-route.json"
    gh_output = tmp_path / "github-output"
    write_json(premerge, {"schema_version": "fix-merge-premerge-report.v1", "clean": True, "results": []})
    write_json(collection, {"schema_version": "fix-dispatch-collection-result.v1", "results": []})
    write_json(pr_context, {"head_sha": "abc123"})
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))

    assert main([
        "fix_merge",
        "prepare-merge-model",
        "--inventory",
        str(premerge),
        "--in",
        str(collection),
        "--pr-context",
        str(pr_context),
        "--raw-out",
        str(raw_out),
        "--out",
        str(route),
    ]) == 0

    assert json.loads(route.read_text(encoding="utf-8"))["needs_model"] is False
    assert json.loads(raw_out.read_text(encoding="utf-8"))["status"] == "no_fix"
    assert "needs_model=false" in gh_output.read_text(encoding="utf-8")


def test_fix_merge_prepare_merge_model_preserves_clean_new_file_patch(tmp_path, monkeypatch):
    repo = tmp_path / "repo"
    repo.mkdir()
    subprocess.run(["git", "init"], cwd=repo, check=True, capture_output=True)
    subprocess.run(["git", "config", "user.email", "codex-review@example.invalid"], cwd=repo, check=True)
    subprocess.run(["git", "config", "user.name", "Codex Review"], cwd=repo, check=True)
    (repo / "README.md").write_text("base\n", encoding="utf-8")
    subprocess.run(["git", "add", "README.md"], cwd=repo, check=True)
    subprocess.run(["git", "commit", "-m", "base"], cwd=repo, check=True, capture_output=True)

    patch = (
        "diff --git a/docs/CODEX_REVIEW_LGTM_LOOP.md b/docs/CODEX_REVIEW_LGTM_LOOP.md\n"
        "new file mode 100644\n"
        "index 0000000..7898192\n"
        "--- /dev/null\n"
        "+++ b/docs/CODEX_REVIEW_LGTM_LOOP.md\n"
        "@@ -0,0 +1 @@\n"
        "+LGTM loop\n"
    )
    collection = tmp_path / "collection.json"
    premerge = tmp_path / "premerge.json"
    pr_context = tmp_path / "pr-context.json"
    raw_out = tmp_path / "merged-fix.raw.json"
    route = tmp_path / "merge-route.json"
    write_json(
        collection,
        {
            "schema_version": "fix-dispatch-collection-result.v1",
            "results": [
                {
                    "schema_version": "fix-dispatch-agent-result.v1",
                    "task_id": "correctness-001",
                    "status": "patched",
                    "patch": patch,
                }
            ],
        },
    )
    write_json(pr_context, {"head_sha": "abc123"})

    assert main(["fix_merge", "premerge", "--in", str(collection), "--repo-path", str(repo), "--out", str(premerge)]) == 0
    assert json.loads(premerge.read_text(encoding="utf-8"))["clean"] is True

    gh_output = tmp_path / "github-output"
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))
    assert main([
        "fix_merge",
        "prepare-merge-model",
        "--inventory",
        str(premerge),
        "--in",
        str(collection),
        "--pr-context",
        str(pr_context),
        "--raw-out",
        str(raw_out),
        "--out",
        str(route),
    ]) == 0

    merged = json.loads(raw_out.read_text(encoding="utf-8"))
    assert merged["patch"] == patch
    assert "docs/CODEX_REVIEW_LGTM_LOOP.md" in merged["patch"]
    assert json.loads(route.read_text(encoding="utf-8"))["needs_model"] is False
    assert "needs_model=false" in gh_output.read_text(encoding="utf-8")


def test_fix_merge_prepare_merge_model_writes_prompt_for_conflicts(tmp_path, monkeypatch):
    premerge = tmp_path / "premerge.json"
    collection = tmp_path / "collection.json"
    pr_context = tmp_path / "pr-context.json"
    docs = tmp_path / "docs.md"
    raw_out = tmp_path / "merged-fix.raw.json"
    prompt_out = tmp_path / "merge-prompt.md"
    route = tmp_path / "merge-route.json"
    gh_output = tmp_path / "github-output"
    write_json(premerge, {"schema_version": "fix-merge-premerge-report.v1", "clean": False, "results": [{"index": 0, "ok": False}]})
    write_json(collection, {"schema_version": "fix-dispatch-collection-result.v1", "results": [{"task_id": "t1", "status": "patched", "patch": "diff --git a/x b/x\n"}]})
    write_json(pr_context, {"head_sha": "abc123"})
    docs.write_text("docs", encoding="utf-8")
    monkeypatch.setenv("GITHUB_OUTPUT", str(gh_output))

    assert main([
        "fix_merge",
        "prepare-merge-model",
        "--inventory",
        str(premerge),
        "--in",
        str(collection),
        "--pr-context",
        str(pr_context),
        "--docs-context",
        str(docs),
        "--prompt-out",
        str(prompt_out),
        "--raw-out",
        str(raw_out),
        "--out",
        str(route),
    ]) == 0

    assert json.loads(route.read_text(encoding="utf-8"))["needs_model"] is True
    assert prompt_out.is_file()
    assert not raw_out.exists()
    assert "fix-merge-merged-fix.v1" in prompt_out.read_text(encoding="utf-8")
    assert "needs_model=true" in gh_output.read_text(encoding="utf-8")
