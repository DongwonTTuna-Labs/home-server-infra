"""CLI handler: fix_merge commands."""
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


def handle_fix_merge(args: argparse.Namespace, config: dict[str, Any]) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "premerge":
        from codex_review.stages.fix_merge.premerge import run_premerge_check
        return run_premerge_check(_maybe_json(args.in_path, {}), args.repo_path), "fix-merge-premerge-report.v1"
    if cmd == "merge":
        from codex_review.stages.fix_merge.premerge import create_merged_fix_from_premerge
        return create_merged_fix_from_premerge(_maybe_json(args.inventory, {}), _maybe_json(args.in_path, {}), _maybe_json(args.pr_context, {}), args.out), "fix-merge-merged-fix.v1"
    if cmd == "model-merged-fix":
        from codex_review.model.adapter import run_model_or_fallback, write_prompt_if_needed
        from codex_review.stages.fix_merge.premerge import create_merged_fix_from_premerge
        from codex_review.stages.fix_merge.prompt import build_fix_merge_prompt
        pre = _maybe_json(args.inventory, {})
        collection = _maybe_json(args.in_path, {})
        pr = _maybe_json(args.pr_context, {})
        if pre.get("clean") or not collection.get("results"):
            return create_merged_fix_from_premerge(pre, collection, pr, None), "fix-merge-merged-fix.v1"
        fallback = {"schema_version":"fix-merge-merged-fix.v1", "status":"blocked", "patch":"", "expected_head_sha": pr.get("head_sha"), "premerge_clean": False, "conflicts": pre.get("results", []), "defaulted": True}
        if not args.prompt:
            args.prompt = str(write_prompt_if_needed(build_fix_merge_prompt(collection, pre, pr, {}, _maybe_text(args.docs_context)), args.out))
        return run_model_or_fallback(stage="fix_merge_merge", prompt_path=args.prompt, output_path=args.out, expected_schema="fix-merge-merged-fix.v1", fallback=fallback, model_command=args.model_command, cwd=args.model_cwd, target_repo_path=args.repo_path), "fix-merge-merged-fix.v1"
    if cmd == "build-merge-prompt":
        from codex_review.stages.fix_merge.prompt import build_fix_merge_prompt
        return build_fix_merge_prompt(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {}), _maybe_json(args.result, {}), {}, _maybe_text(args.docs_context)), None
    if cmd == "prepare-merge-model":
        from codex_review.stages.fix_merge.premerge import create_merged_fix_from_premerge
        from codex_review.stages.fix_merge.prompt import build_fix_merge_prompt
        pre = _maybe_json(args.inventory, {})
        collection = _maybe_json(args.in_path, {})
        pr = _maybe_json(args.pr_context, {})
        raw_out = args.raw_out or args.result
        prompt_out = args.prompt_out or args.prompt
        if pre.get("clean") or not collection.get("results"):
            merged = create_merged_fix_from_premerge(pre, collection, pr, raw_out)
            if raw_out and not Path(raw_out).exists():
                write_json(raw_out, merged, "fix-merge-merged-fix.v1")
            route = {"needs_model": False, "raw_output": raw_out, "status": merged.get("status")}
            if os.environ.get("GITHUB_OUTPUT"):
                write_output("needs_model", "false")
            return route, None
        if not prompt_out:
            raise ValidationError("prepare-merge-model requires --prompt-out when model merge is needed")
        write_text(prompt_out, build_fix_merge_prompt(pre, collection, {}, {}, {"pr_context": pr, "docs_context": _maybe_text(args.docs_context)}))
        route = {"needs_model": True, "prompt": prompt_out, "raw_output": raw_out}
        if os.environ.get("GITHUB_OUTPUT"):
            write_output("needs_model", "true")
        return route, None
    if cmd == "default-merged-fix":
        pre = _maybe_json(args.inventory or args.in_path, {})
        return {"schema_version": "fix-merge-merged-fix.v1", "status": "no_fix", "patch": "", "premerge_clean": pre.get("clean", False), "defaulted": True}, "fix-merge-merged-fix.v1"
    if cmd == "validate":
        from codex_review.stages.fix_merge.validate import validate_merged_fix
        from codex_review.patches.fix_edits import ensure_patch_from_edits
        pre = _maybe_json(args.inventory, {})
        chief = _maybe_json(args.result or args.chief_decision, {})
        merged = ensure_patch_from_edits(_maybe_json(args.in_path, {}), args.repo_path)
        return validate_merged_fix(merged, pre, chief, config.get("autofix", {}), args.repo_path), "fix-merge-merged-fix.v1"
    if cmd == "build-semantic-safety-prompt":
        from codex_review.stages.fix_merge.semantic_safety import build_semantic_patch_safety_prompt
        prompt = build_semantic_patch_safety_prompt(
            _maybe_json(args.in_path, {}),
            _maybe_json(args.pr_context, {}),
            _maybe_text(args.docs_context, ""),
            repo_path=args.repo_path,
            token_budget=int((config.get("context", {}) or {}).get("model_token_budget", 0)) or None,
        )
        return prompt, None
    if cmd == "validate-semantic-safety":
        from codex_review.stages.fix_merge.semantic_safety import validate_semantic_patch_safety_result
        return validate_semantic_patch_safety_result(_maybe_json(args.in_path, {}), _maybe_json(args.inventory, {})), "fix-merge-semantic-patch-safety.v1"
    if cmd == "write-semantic-safety-outputs":
        from codex_review.stages.fix_merge.semantic_safety import write_semantic_safety_outputs
        return write_semantic_safety_outputs(_maybe_json(args.in_path, {})), None
    if cmd == "render":
        from codex_review.stages.fix_merge.render import render_merged_fix_summary
        return render_merged_fix_summary(_maybe_json(args.in_path, {})), None
    raise ValueError(f"unknown fix_merge command: {cmd}")
