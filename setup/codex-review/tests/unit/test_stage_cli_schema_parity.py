"""Stage CLI/action schema-parity tests (Task 19).

These tests prove that the reusable Codex-loop workflow's deterministic,
model-free stage commands (the same trusted ``codex-review`` CLI entrypoints
wired by Tasks 13-15) produce artifacts whose *shapes* conform to the existing
``schemas/*.v1.schema.json`` contracts the legacy split workflow relied on.

Why schema-shape parity and not byte-exact legacy output:

* Wave 3 runs every stage in ``dry_run`` (the default). ``setup-relay`` never
  mints the OIDC relay token, so no model executes; each stage falls back to the
  purpose-built deterministic ``default-*`` CLI commands. Exact legacy *content*
  is produced only when a live model runs (a later live-capable change), so the
  reproducible parity guarantee for Wave 3 is that the deterministic outputs
  still satisfy the same v1 schema contracts.

The tests drive the real CLI through ``codex_review.cli.main([...])`` (the same
``codex-review`` console entrypoint the workflow invokes) and validate every
emitted artifact against its on-disk JSON Schema with ``jsonschema``. They are
deterministic and perform no network, GitHub, model, or push calls.

Scope note (issue stage): the issue stage is an artifact-only terminal fallback
introduced in Task 16 and is intentionally NOT part of review/design/fix legacy
v1 schema parity. ``test_issue_stage_is_artifact_only_terminal_and_out_of_legacy_schema_parity``
documents and pins that boundary explicitly rather than omitting it silently.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import jsonschema

from codex_review.cli import main
from codex_review.core.schema import load_schema_json


REVIEW_AXES = ("correctness", "security", "performance", "test-coverage", "domain")

_EVIDENCE = [{"path": "AGENTS.md", "purpose": "parity fixture", "observation": "deterministic stage parity test"}]


def _write_json(path: Path, payload: dict[str, Any]) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, ensure_ascii=False, sort_keys=True), encoding="utf-8")
    return path


def _run(*args: str) -> int:
    return main(list(args))


def _run_to_json(out_path: Path, *args: str) -> dict[str, Any]:
    rc = main([*args, "--out", str(out_path)])
    assert rc == 0, f"codex-review {' '.join(args)} exited {rc}"
    payload = json.loads(out_path.read_text(encoding="utf-8"))
    assert isinstance(payload, dict)
    return payload


def _assert_schema(payload: dict[str, Any], schema_name: str) -> None:
    """Validate against the on-disk v1 schema, failing loudly if jsonschema is absent.

    Using ``jsonschema.validate`` directly (rather than the library's
    ``validate_json_schema`` helper, which silently no-ops when jsonschema is
    missing) makes the schema check a hard gate for the parity proof.
    """
    jsonschema.validate(payload, load_schema_json(schema_name))
    assert payload.get("schema_version") == schema_name


# --------------------------------------------------------------------------- #
# Review stage parity                                                          #
# --------------------------------------------------------------------------- #

def test_review_default_result_parity_conforms_to_axis_schema(tmp_path: Path) -> None:
    """Each of the 5 review axes' deterministic ``review default-result`` output
    conforms to review-axis-findings.v1 (the matrix axes the workflow runs)."""
    for axis in REVIEW_AXES:
        payload = _run_to_json(tmp_path / f"axis-{axis}.json", "review", "default-result", "--axis", axis)
        _assert_schema(payload, "review-axis-findings.v1")
        assert payload["axis"] == axis
        assert payload["findings"] == []
        assert payload["defaulted"] is True


def test_review_combine_techlead_classify_parity_empty_chain(tmp_path: Path) -> None:
    """The deterministic review pipeline (default-result x5 -> combine ->
    techlead default-result -> classify) is schema-valid end to end and routes
    to LGTM when no findings exist (the dry-run default outcome)."""
    artifacts_dir = tmp_path / "axes"
    for axis in REVIEW_AXES:
        axis_payload = _run_to_json(artifacts_dir / axis / "findings.json", "review", "default-result", "--axis", axis)
        _assert_schema(axis_payload, "review-axis-findings.v1")

    combined = _run_to_json(tmp_path / "combined.json", "review", "combine", "--artifacts", str(artifacts_dir))
    _assert_schema(combined, "review-combined-findings.v1")
    assert combined["finding_count"] == 0
    assert combined["findings"] == []

    techlead = _run_to_json(tmp_path / "techlead.json", "techlead", "default-result", "--in", str(tmp_path / "combined.json"))
    _assert_schema(techlead, "techlead-decision.v1")
    assert techlead["decisions"] == []
    assert techlead["status"] == "lgtm"

    publication = _run_to_json(
        tmp_path / "publication.json",
        "techlead", "classify",
        "--in", str(tmp_path / "techlead.json"),
        "--inventory", str(tmp_path / "combined.json"),
    )
    _assert_schema(publication, "techlead-review-publication.v1")
    assert publication["status"] == "lgtm"
    assert publication["inline_comments"] == []


def test_review_combine_techlead_classify_parity_populated_chain(tmp_path: Path) -> None:
    """With real per-axis findings flowing through the trusted CLI, combine ->
    techlead default-result -> classify still conform to their v1 schemas and
    carry the findings forward (exercises the non-empty merge/classify paths)."""
    artifacts_dir = tmp_path / "axes"
    axis_findings = {
        "correctness": {"finding_id": "F-correctness-1", "severity": "high", "file": "src/lib.rs", "line": 10, "title": "off-by-one", "summary": "loop bound", "root_cause_key": "loop-bound"},
        "security": {"finding_id": "F-security-1", "severity": "critical", "file": "src/auth.rs", "line": 4, "title": "missing authz", "summary": "no check", "root_cause_key": "authz"},
    }
    for axis, finding in axis_findings.items():
        _write_json(
            artifacts_dir / axis / "findings.json",
            {
                "schema_version": "review-axis-findings.v1",
                "axis": axis,
                "finding_count": 1,
                "findings": [dict(finding, axis=axis)],
                "inspection_evidence": _EVIDENCE,
            },
        )

    combined = _run_to_json(tmp_path / "combined.json", "review", "combine", "--artifacts", str(artifacts_dir))
    _assert_schema(combined, "review-combined-findings.v1")
    assert combined["finding_count"] == 2
    combined_ids = {f.get("finding_id") for f in combined["findings"]}
    assert combined_ids == {"F-correctness-1", "F-security-1"}

    techlead = _run_to_json(tmp_path / "techlead.json", "techlead", "default-result", "--in", str(tmp_path / "combined.json"))
    _assert_schema(techlead, "techlead-decision.v1")
    assert {d["finding_id"] for d in techlead["decisions"]} == {"F-correctness-1", "F-security-1"}
    assert all(d["action"] == "publish_only" for d in techlead["decisions"])

    publication = _run_to_json(
        tmp_path / "publication.json",
        "techlead", "classify",
        "--in", str(tmp_path / "techlead.json"),
        "--inventory", str(tmp_path / "combined.json"),
    )
    _assert_schema(publication, "techlead-review-publication.v1")
    assert len(publication["inline_comments"]) == 2
    assert publication["status"] in {"publishable", "ready"}


# --------------------------------------------------------------------------- #
# Design stage parity                                                          #
# --------------------------------------------------------------------------- #

def _design_chain(tmp_path: Path, *, techlead_path: Path | None, openspec_path: Path | None) -> dict[str, dict[str, Any]]:
    """Run the deterministic design chain (context -> inventory -> clusters ->
    default-analysis -> default-plan -> chief default-result -> chief route) and
    return the validated artifacts keyed by stage."""
    context_args = ["design", "context"]
    if techlead_path is not None:
        context_args += ["--in", str(techlead_path)]
    if openspec_path is not None:
        context_args += ["--openspec-context", str(openspec_path)]
    context = _run_to_json(tmp_path / "design-context.json", *context_args)
    _assert_schema(context, "design-context.v1")

    inventory = _run_to_json(tmp_path / "design-inventory.json", "design", "default-inventory", "--in", str(tmp_path / "design-context.json"))
    _assert_schema(inventory, "design-inventory.v1")

    clusters = _run_to_json(tmp_path / "design-clusters.json", "design", "default-clusters", "--inventory", str(tmp_path / "design-inventory.json"))
    _assert_schema(clusters, "design-clusters.v1")

    analysis = _run_to_json(tmp_path / "design-analysis.json", "design", "default-analysis", "--inventory", str(tmp_path / "design-clusters.json"))
    _assert_schema(analysis, "design-cluster-analysis.v1")

    plan = _run_to_json(tmp_path / "design-plan.json", "design", "default-plan", "--pr-context", str(tmp_path / "design-context.json"))
    _assert_schema(plan, "design-plan.v1")

    chief = _run_to_json(tmp_path / "design-chief.json", "design_chief", "default-result", "--design-plan", str(tmp_path / "design-plan.json"))
    _assert_schema(chief, "design-chief-decision.v1")

    route = _run_to_json(tmp_path / "design-route.json", "design_chief", "route", "--in", str(tmp_path / "design-chief.json"))
    return {"context": context, "inventory": inventory, "clusters": clusters, "analysis": analysis, "plan": plan, "chief": chief, "route": route}


def test_design_chain_parity_empty(tmp_path: Path) -> None:
    """Empty (no prior findings) deterministic design chain is schema-valid and
    terminates as no_fix_needed -> stop_noop (the dry-run default outcome)."""
    artifacts = _design_chain(tmp_path, techlead_path=None, openspec_path=None)
    assert artifacts["context"]["findings"] == []
    assert artifacts["inventory"]["items"] == []
    assert artifacts["clusters"]["clusters"] == []
    assert artifacts["analysis"]["analyses"] == []
    assert artifacts["plan"]["edit_sequence"] == []
    assert artifacts["chief"]["status"] == "no_fix_needed"
    assert artifacts["route"]["route"] == "stop_noop"


def test_design_chain_parity_populated_openspec(tmp_path: Path) -> None:
    """With a techlead decision that needs design plus OpenSpec context, the
    deterministic design chain produces a non-empty, schema-valid design-plan.v1
    and design-chief-decision.v1 and routes to a human checkpoint (deterministic
    chief never auto-approves without a model)."""
    techlead_path = _write_json(
        tmp_path / "in-techlead.json",
        {
            "schema_version": "techlead-decision.v1",
            "decisions": [
                {"finding_id": "F-1", "action": "publish_and_fix_now", "file": "src/lib.rs", "summary": "Guard null deref", "root_cause_key": "null-deref"}
            ],
            "needs_design": True,
            "status": "needs_design",
            "inspection_evidence": _EVIDENCE,
        },
    )
    openspec_path = _write_json(
        tmp_path / "in-openspec.json",
        {"present": True, "source_summary": ["openspec/changes/demo/tasks.md"]},
    )

    artifacts = _design_chain(tmp_path, techlead_path=techlead_path, openspec_path=openspec_path)
    assert artifacts["context"]["findings"], "design context should carry the needs-design finding forward"
    assert artifacts["context"]["openspec_backed"] is True
    assert artifacts["inventory"]["items"]
    assert artifacts["clusters"]["clusters"]
    assert artifacts["plan"]["edit_sequence"], "populated plan must contain an edit_sequence"
    assert artifacts["plan"]["openspec_backed"] is True
    assert artifacts["chief"]["status"] in {"needs_human", "approved_for_fix", "no_fix_needed", "rejected_plan"}
    assert artifacts["chief"]["status"] == "needs_human"
    assert artifacts["route"]["route"] == "stop_needs_human"


# --------------------------------------------------------------------------- #
# Fix stage parity                                                             #
# --------------------------------------------------------------------------- #

def test_fix_dispatch_chain_parity_deterministic(tmp_path: Path) -> None:
    """The deterministic fix-dispatch chain (plan -> prepare-agents ->
    default-agent-result -> validate-agent-result -> collect) conforms to its v1
    schemas without any model, relay token, or push."""
    design_plan_path = _write_json(
        tmp_path / "design-plan.json",
        {
            "schema_version": "design-plan.v1",
            "edit_sequence": [
                {
                    "task_id": "fix-1",
                    "summary": "Add a bounds check",
                    "allowed_files": ["src/lib.rs"],
                    "finding_ids": ["F-1"],
                    "acceptance_criteria": ["covered by a regression test"],
                }
            ],
            "tests": ["cargo test --workspace"],
            "acceptance_criteria": ["covered by a regression test"],
            "inspection_evidence": _EVIDENCE,
            "openspec_backed": False,
            "plan_hash": "deadbeefcafe",
        },
    )
    chief_path = _write_json(
        tmp_path / "chief.json",
        {
            "schema_version": "design-chief-decision.v1",
            "status": "approved_for_fix",
            "fix_policy": {"allowed_files": ["src/lib.rs"]},
            "inspection_evidence": _EVIDENCE,
        },
    )

    manifest = _run_to_json(
        tmp_path / "manifest.json",
        "fix_dispatch", "plan",
        "--in", str(design_plan_path),
        "--inventory", str(chief_path),
    )
    _assert_schema(manifest, "fix-dispatch-task-manifest.v1")
    assert manifest["tasks"], "approved design plan with an edit_sequence yields fix tasks"
    task = manifest["tasks"][0]

    matrix = _run_to_json(
        tmp_path / "matrix.json",
        "fix_dispatch", "prepare-agents",
        "--inventory", str(tmp_path / "manifest.json"),
        "--work-dir", str(tmp_path / "agents-work"),
    )
    assert len(matrix["include"]) == len(manifest["tasks"])
    assert matrix["include"][0]["task_id"] == task["task_id"]

    task_path = _write_json(tmp_path / "task.json", task)
    agent_result = _run_to_json(
        tmp_path / "agent-result.json",
        "fix_dispatch", "default-agent-result",
        "--inventory", str(task_path),
    )
    _assert_schema(agent_result, "fix-dispatch-agent-result.v1")
    assert agent_result["task_id"] == task["task_id"]
    assert agent_result["status"] == "no_safe_fix"

    validated_agent = _run_to_json(
        tmp_path / "results" / task["task_id"] / "result.validated.json",
        "fix_dispatch", "validate-agent-result",
        "--in", str(tmp_path / "agent-result.json"),
        "--inventory", str(task_path),
    )
    _assert_schema(validated_agent, "fix-dispatch-agent-result.v1")
    assert validated_agent["status"] == "no_safe_fix"

    collection = _run_to_json(
        tmp_path / "collection.json",
        "fix_dispatch", "collect",
        "--inventory", str(tmp_path / "manifest.json"),
        "--artifacts", str(tmp_path / "results"),
    )
    _assert_schema(collection, "fix-dispatch-collection-result.v1")
    assert collection["ready_for_merge"] is False
    assert {r.get("task_id") for r in collection["results"]} == {task["task_id"]}


def test_fix_merge_and_push_validate_dry_run_parity(tmp_path: Path) -> None:
    """``fix_merge default-merged-fix`` and ``push validate-fix --dry-run`` (the
    deterministic, no-push Wave-3 path) emit fix-merge-merged-fix.v1 and
    push-validated-fix.v1 with ``pushed: false`` and no commit/push side effect.

    Dry-run scope: the deterministic merge yields ``status: no_fix`` (no model
    patch is produced in Wave 3), so push validation short-circuits to a
    no-push, unvalidated artifact. A populated ``status: dry_run`` validation
    requires a model-produced patch applied in a PR-head git checkout, which is
    deferred to the live-capable change (Task 13/15 OPEN boundary)."""
    merged = _run_to_json(tmp_path / "merged-fix.json", "fix_merge", "default-merged-fix")
    _assert_schema(merged, "fix-merge-merged-fix.v1")
    assert merged["status"] == "no_fix"

    validated = _run_to_json(tmp_path / "validated.json", "push", "validate-fix", "--dry-run", "--in", str(tmp_path / "merged-fix.json"))
    _assert_schema(validated, "push-validated-fix.v1")
    assert validated["pushed"] is False
    assert validated["validated"] is False
    assert validated["status"] == "no_fix"

    budget = _run_to_json(tmp_path / "budget.json", "push", "check-loop-budget", "--in", str(tmp_path / "validated.json"))
    _assert_schema(budget, "push-validated-fix.v1")
    assert budget["pushed"] is False
    assert budget["loop_budget_ok"] is True


# --------------------------------------------------------------------------- #
# Issue stage scope (explicit, not silently omitted)                           #
# --------------------------------------------------------------------------- #

def test_issue_stage_is_artifact_only_terminal_and_out_of_legacy_schema_parity(tmp_path: Path) -> None:
    """Pin the issue-stage parity boundary explicitly.

    The issue stage (Task 16) is an artifact-only terminal fallback: in Wave 3
    it runs ``issue_fallback infer-reason/plan/compose`` to produce a terminal
    issue artifact and a step summary, but it never calls ``apply``, never gains
    ``issues: write``, and is NOT part of the review/design/fix legacy v1 schema
    parity surface. Its deterministic ``plan`` artifact is emitted under
    ``issue-fallback-issue-fallback.v1`` (a terminal-fallback contract), which is
    deliberately distinct from the review/design/fix legacy schemas exercised
    above. This test documents and locks that boundary rather than omitting it.
    """
    pr_context_path = _write_json(
        tmp_path / "pr-context.json",
        {"owner": "o", "repo": "r", "repository": "o/r", "pr_number": 7},
    )
    plan = _run_to_json(
        tmp_path / "issue-plan.json",
        "issue_fallback", "plan",
        "--mode", "artifacts_missing",
        "--pr-context", str(pr_context_path),
    )

    # Artifact-only terminal fallback: a self-contained, idempotent issue plan.
    assert plan["schema_version"] == "issue-fallback-issue-fallback.v1"
    assert plan["idempotency_key"]
    assert plan["title"]
    assert plan["body"]
    assert "artifacts_missing" in plan["title"]

    # The issue terminal contract is deliberately distinct from the
    # review/design/fix legacy v1 schemas asserted by the parity tests above.
    legacy_parity_schemas = {
        "review-axis-findings.v1",
        "review-combined-findings.v1",
        "techlead-decision.v1",
        "techlead-review-publication.v1",
        "design-context.v1",
        "design-inventory.v1",
        "design-clusters.v1",
        "design-cluster-analysis.v1",
        "design-plan.v1",
        "design-chief-decision.v1",
        "fix-dispatch-task-manifest.v1",
        "fix-dispatch-agent-result.v1",
        "fix-dispatch-collection-result.v1",
        "fix-merge-merged-fix.v1",
        "push-validated-fix.v1",
    }
    assert plan["schema_version"] not in legacy_parity_schemas
