"""CLI handler: design commands."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, read_text, write_json, write_text
from codex_review.core.config import load_config
from codex_review.core.env import read_event_payload
from codex_review.core.errors import CodexReviewError, ValidationError, format_error
from codex_review.core.output import append_step_summary, mask_secret, write_output
from codex_review.cli._helpers import (
    _add_common, _artifact_paths, _default_inspection_evidence, _emit,
    _json_or_default, _maybe_json, _maybe_text, _model_or_fallback,
    _preferred_artifact_paths, _repo_parts_from_context, _safe_path_component,
)


def handle_design(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "context":
        from codex_review.stages.design.context import build_design_context
        return build_design_context(_maybe_json(args.pr_context, {}), _maybe_json(args.in_path, {}), _maybe_text(args.review_context), _maybe_text(args.docs_context), None, _json_or_default(args.openspec_context, {})), "design-context.v1"
    if cmd == "build-inventory-prompt":
        from codex_review.stages.design.normalize import build_normalize_prompt
        memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
        return build_normalize_prompt(_maybe_json(args.in_path or args.inventory, {}), memory_context=memory_context), None
    if cmd in {"default-inventory", "model-inventory"}:
        ctx = _maybe_json(args.in_path, {})
        items = []
        for finding in ctx.get("findings", []):
            items.append({"finding_id": finding.get("finding_id"), "invariant": finding.get("root_cause_key") or finding.get("title") or finding.get("finding_id"), "summary": finding.get("summary", "")})
        fallback = {
            "schema_version": "design-inventory.v1",
            "items": items,
            "item_count": len(items),
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design inventory model result was available."),
            "defaulted": True,
        }
        if cmd == "model-inventory":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.design.normalize import build_normalize_prompt
                memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
                args.prompt = str(write_prompt_if_needed(build_normalize_prompt(ctx, memory_context=memory_context), args.out))
            return _model_or_fallback(args, stage="design_inventory", expected_schema="design-inventory.v1", fallback=fallback), "design-inventory.v1"
        return fallback, "design-inventory.v1"
    if cmd == "normalize":
        from codex_review.stages.design.normalize import validate_design_inventory
        ctx = _maybe_json(args.pr_context or args.inventory, {})
        tech = ctx.get("techlead_decision", ctx)
        return validate_design_inventory(_maybe_json(args.in_path, {}), tech, args.repo_path), "design-inventory.v1"
    if cmd == "build-clusters-prompt":
        from codex_review.stages.design.cluster import build_cluster_prompt
        memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
        return build_cluster_prompt(_maybe_json(args.inventory or args.in_path, {}), _maybe_json(args.pr_context, {}), memory_context=memory_context), None
    if cmd in {"default-clusters", "model-clusters"}:
        inv = _maybe_json(args.inventory or args.in_path, {})
        clusters = []
        for idx, item in enumerate(inv.get("items", []), 1):
            clusters.append({"cluster_id": f"cluster-{idx}", "finding_ids": [item.get("finding_id")], "summary": item.get("summary", "")})
        fallback = {
            "schema_version": "design-clusters.v1",
            "clusters": clusters,
            "cluster_count": len(clusters),
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design cluster model result was available."),
            "defaulted": True,
        }
        if cmd == "model-clusters":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.design.cluster import build_cluster_prompt
                memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
                args.prompt = str(write_prompt_if_needed(build_cluster_prompt(inv, _maybe_json(args.pr_context, {}), memory_context=memory_context), args.out))
            return _model_or_fallback(args, stage="design_clusters", expected_schema="design-clusters.v1", fallback=fallback), "design-clusters.v1"
        return fallback, "design-clusters.v1"
    if cmd == "cluster":
        from codex_review.stages.design.cluster import validate_design_clusters
        return validate_design_clusters(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {}), args.repo_path), "design-clusters.v1"
    if cmd == "batch":
        from codex_review.stages.design.batch import make_cluster_batches
        return {"batches": make_cluster_batches(_maybe_json(args.in_path, {}), config)}, None
    if cmd == "prepare-analysis-matrix":
        from codex_review.stages.design.analyze import build_cluster_analysis_prompt
        from codex_review.stages.design.batch import make_cluster_batches
        clusters = _maybe_json(args.in_path or args.inventory, {})
        design_context = _maybe_json(args.pr_context, {})
        batches = make_cluster_batches(clusters, config)
        base_dir = Path(args.work_dir) if args.work_dir else Path("codex-review-artifacts/design/batches")
        base_dir.mkdir(parents=True, exist_ok=True)
        include = []
        for batch in batches:
            if not batch.get("clusters"):
                continue
            batch_index = int(batch.get("batch_index", len(include)))
            batch_path = str(batch_index)
            batch_dir = base_dir / batch_path
            batch_dir.mkdir(parents=True, exist_ok=True)
            batch_file = batch_dir / "batch.json"
            prompt_file = batch_dir / "analysis.prompt.md"
            output_file = batch_dir / "analysis.raw.json"
            validated_file = batch_dir / "analysis.validated.json"
            write_json(batch_file, batch, "design-cluster-batch.v1")
            write_text(prompt_file, build_cluster_analysis_prompt(batch, design_context))
            include.append(
                {
                    "batch_index": batch_index,
                    "batch_path": batch_path,
                    "batch_file": batch_file.as_posix(),
                    "prompt_file": prompt_file.as_posix(),
                    "output_file": output_file.as_posix(),
                    "validated_file": validated_file.as_posix(),
                    "working_directory": args.repo_path,
                }
            )
        matrix = {"include": include}
        if os.environ.get("GITHUB_OUTPUT"):
            write_output("has_analysis_batches", "true" if include else "false")
            write_output("analysis_matrix", json.dumps(matrix, sort_keys=True, separators=(",", ":")))
        return matrix, None
    if cmd == "collect-analyses":
        from codex_review.stages.design.analyze import combine_cluster_analyses
        paths = _preferred_artifact_paths(args.artifacts, primary="analysis.validated.json", fallback="*.json")
        analyses = combine_cluster_analyses(paths)
        return {"schema_version": "design-cluster-analysis.v1", "analyses": analyses, "analysis_count": len(analyses)}, "design-cluster-analysis.v1"
    if cmd == "build-analysis-prompt":
        from codex_review.stages.design.analyze import build_cluster_analysis_prompt
        return build_cluster_analysis_prompt(_maybe_json(args.inventory or args.in_path, {}), _maybe_json(args.pr_context, {})), None
    if cmd in {"default-analysis", "model-analysis"}:
        clusters = _maybe_json(args.inventory or args.in_path, {})
        analyses = [{"cluster_id": c.get("cluster_id"), "status": "needs_human", "recommendation": "model analysis not provided"} for c in clusters.get("clusters", [])]
        fallback = {
            "schema_version": "design-cluster-analysis.v1",
            "analyses": analyses,
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No cluster analysis model result was available."),
            "defaulted": True,
        }
        if cmd == "model-analysis":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.design.analyze import build_cluster_analysis_prompt
                args.prompt = str(write_prompt_if_needed(build_cluster_analysis_prompt(clusters, _maybe_json(args.pr_context, {})), args.out))
            return _model_or_fallback(args, stage="design_analysis", expected_schema="design-cluster-analysis.v1", fallback=fallback), "design-cluster-analysis.v1"
        return fallback, "design-cluster-analysis.v1"
    if cmd == "analyze":
        from codex_review.stages.design.analyze import validate_cluster_analysis
        batch = _maybe_json(args.artifacts[0], {}) if args.artifacts else _maybe_json(args.inventory, {})
        return validate_cluster_analysis(_maybe_json(args.in_path, {}), batch, args.repo_path), "design-cluster-analysis.v1"
    if cmd in {"default-plan", "model-plan"}:
        from codex_review.stages.design.coordinate import validate_design_plan
        ctx = _maybe_json(args.pr_context or args.inventory, {})
        findings = ctx.get("findings", [])
        plan = {
            "schema_version": "design-plan.v1",
            "edit_sequence": [],
            "tests": [],
            "inspection_evidence": _default_inspection_evidence("deterministic fallback", "No design plan model result was available."),
            "defaulted": True,
        }
        if findings:
            plan["openspec_backed"] = bool(ctx.get("openspec_backed"))
            plan["edit_sequence"] = [
                {
                    "task_id": f"openspec-fallback-{idx}" if ctx.get("openspec_backed") else f"manual-design-{idx}",
                    "finding_ids": [finding.get("finding_id") or finding.get("id")],
                    "summary": finding.get("summary") or "Implement the OpenSpec-backed finding conservatively.",
                    "allowed_files": finding.get("files") or ([finding.get("file")] if finding.get("file") else []),
                    "acceptance_criteria": ["OpenSpec tasks and affected tests pass"] if ctx.get("openspec_backed") else [],
                }
                for idx, finding in enumerate(findings, 1)
            ]
            plan["tests"] = ["cargo fmt --all --check", "cargo test --workspace --all-features"] if ctx.get("openspec_backed") else ["Human design review required before automated tests are selected"]
            if not ctx.get("openspec_backed"):
                plan["requires_human_review"] = True
            fallback = validate_design_plan(plan, ctx, config)
        else:
            fallback = validate_design_plan(plan, ctx, config)
        if cmd == "model-plan":
            if not args.prompt:
                from codex_review.model.adapter import write_prompt_if_needed
                from codex_review.stages.design.coordinate import build_coordinate_prompt
                memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
                args.prompt = str(write_prompt_if_needed(build_coordinate_prompt(ctx, _maybe_json(args.inventory, {}), [], memory_context=memory_context), args.out))
            return _model_or_fallback(args, stage="design_plan", expected_schema="design-plan.v1", fallback=fallback), "design-plan.v1"
        return fallback, "design-plan.v1"
    if cmd == "build-plan-prompt":
        from codex_review.stages.design.coordinate import build_coordinate_prompt
        analyses_payload = _json_or_default(args.result, {})
        analyses = analyses_payload.get("analyses", analyses_payload if isinstance(analyses_payload, list) else [])
        memory_context = None if args.memory_context is None else _maybe_text(args.memory_context)
        return build_coordinate_prompt(_maybe_json(args.pr_context, {}), _maybe_json(args.inventory, {}), analyses, memory_context=memory_context), None
    if cmd == "validate-plan":
        from codex_review.stages.design.coordinate import validate_design_plan
        return validate_design_plan(_maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), config, args.repo_path), "design-plan.v1"
    if cmd == "render":
        from codex_review.stages.design.render import render_design_plan_markdown
        return render_design_plan_markdown(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown design command: {cmd}")
