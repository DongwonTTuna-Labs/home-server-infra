"""CLI handler: fix_dispatch commands."""
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


def handle_fix_dispatch(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "plan":
        from codex_review.stages.fix_dispatch.plan import plan_fix_tasks
        return plan_fix_tasks(_maybe_json(args.in_path, {}), _maybe_json(args.inventory or args.chief_decision, {}), config), "fix-dispatch-task-manifest.v1"
    if cmd == "build-agent-prompt":
        from codex_review.stages.fix_dispatch.prompt import build_fix_agent_prompt
        task = _maybe_json(args.in_path, {})
        return build_fix_agent_prompt(task, _maybe_json(args.inventory, {}), _maybe_json(args.result, {}), _maybe_text(args.docs_context), config), None
    if cmd == "prepare-agents":
        from codex_review.stages.fix_dispatch.prompt import build_fix_agent_prompt
        manifest = _maybe_json(args.inventory or args.in_path, {})
        design_plan = _maybe_json(args.design_plan, {})
        chief = _maybe_json(args.chief_decision or args.result, {})
        docs = _maybe_text(args.docs_context)
        base_dir = Path(args.work_dir) if args.work_dir else Path("codex-review-artifacts/fix_dispatch/agents")
        base_dir.mkdir(parents=True, exist_ok=True)
        include = []
        for task in manifest.get("tasks", []):
            task_id = str(task.get("task_id") or f"task-{len(include) + 1}")
            task_path = _safe_path_component(task_id)
            task_dir = base_dir / task_path
            task_dir.mkdir(parents=True, exist_ok=True)
            task_file = task_dir / "task.json"
            prompt_file = task_dir / "prompt.md"
            output_file = task_dir / "result.json"
            validated_file = task_dir / "result.validated.json"
            write_json(task_file, task, None)
            write_text(prompt_file, build_fix_agent_prompt(task, design_plan, chief, docs, config))
            include.append(
                {
                    "task_id": task_id,
                    "task_path": task_path,
                    "task_file": task_file.as_posix(),
                    "prompt_file": prompt_file.as_posix(),
                    "output_file": output_file.as_posix(),
                    "validated_file": validated_file.as_posix(),
                    "working_directory": args.repo_path,
                }
            )
        matrix = {"include": include}
        if os.environ.get("GITHUB_OUTPUT"):
            write_output("has_agent_tasks", "true" if include else "false")
            write_output("agent_matrix", json.dumps(matrix, sort_keys=True, separators=(",", ":")))
        return matrix, None
    if cmd == "default-agent-result":
        task = _maybe_json(args.inventory or args.in_path, {})
        return {"schema_version": "fix-dispatch-agent-result.v1", "task_id": task.get("task_id"), "status": "no_safe_fix", "reason": "model fix result was not provided", "defaulted": True}, "fix-dispatch-agent-result.v1"
    if cmd in {"run-agents", "model-agents"}:
        from codex_review.model.adapter import run_model_or_fallback
        from codex_review.stages.fix_dispatch.collect import build_fix_collection_result
        from codex_review.stages.fix_dispatch.prompt import build_fix_agent_prompt
        from codex_review.stages.fix_dispatch.validate_agent_result import validate_fix_agent_result
        manifest = _maybe_json(args.inventory or args.in_path, {})
        design_plan = _maybe_json(args.design_plan, {})
        chief = _maybe_json(args.chief_decision or args.result, {})
        docs = _maybe_text(args.docs_context)
        base_dir = Path(args.work_dir) if args.work_dir else (Path(args.out).parent / "agents" if args.out else Path("codex-review-artifacts/fix_dispatch/agents"))
        base_dir.mkdir(parents=True, exist_ok=True)
        results=[]
        for task in manifest.get("tasks", []):
            task_id = str(task.get("task_id"))
            task_dir = base_dir / task_id
            task_dir.mkdir(parents=True, exist_ok=True)
            prompt = build_fix_agent_prompt(task, design_plan, chief, docs, config)
            prompt_path = task_dir / "prompt.md"
            write_text(prompt_path, prompt)
            out_path = task_dir / "result.json"
            fallback = {"schema_version":"fix-dispatch-agent-result.v1", "task_id": task_id, "status":"no_safe_fix", "reason":"model fix result was not provided", "defaulted": True}
            raw = run_model_or_fallback(stage=f"fix_dispatch_{task_id}", prompt_path=prompt_path, output_path=out_path, expected_schema="fix-dispatch-agent-result.v1", fallback=fallback, model_command=args.model_command, cwd=args.model_cwd, target_repo_path=args.repo_path)
            validated = validate_fix_agent_result(raw, task, config.get("autofix", {}), args.repo_path)
            write_json(task_dir / "result.validated.json", validated, "fix-dispatch-agent-result.v1")
            results.append(validated)
        return build_fix_collection_result(manifest, results), "fix-dispatch-collection-result.v1"
    if cmd == "validate-agent-result":
        from codex_review.stages.fix_dispatch.validate_agent_result import validate_fix_agent_result
        task = _maybe_json(args.inventory, {})
        return validate_fix_agent_result(_maybe_json(args.in_path, {}), task, config.get("autofix", {}), args.repo_path), "fix-dispatch-agent-result.v1"
    if cmd == "collect":
        from codex_review.stages.fix_dispatch.collect import collect_agent_results
        paths = _artifact_paths(args.artifacts, names=("*.validated.json", "*.json"))
        return collect_agent_results(_maybe_json(args.inventory, {}), paths), "fix-dispatch-collection-result.v1"
    if cmd == "render":
        from codex_review.stages.fix_dispatch.render import render_agent_result_summary
        return render_agent_result_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown fix_dispatch command: {cmd}")
