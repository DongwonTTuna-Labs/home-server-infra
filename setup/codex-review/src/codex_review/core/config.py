"""Config loader and policy accessors."""
from __future__ import annotations

import os
from pathlib import Path
from typing import Any

import yaml

from codex_review.core.errors import ValidationError
from codex_review.core.paths import setup_root

CONFIG_OVERRIDE_ENV = "CODEX_REVIEW_CONFIG_OVERRIDE"

DEFAULT_CONFIG: dict[str, Any] = {
    "base_branch": "main",
    "trusted": {"user": "", "codex_review_authors": []},
    "review": {
        "axes": ["correctness", "security", "performance", "test-coverage", "domain"],
        "max_inline_comments": 12,
        "max_inline_comments_per_file": 3,
        "require_changed_right_line": True,
        "suppress_resolved_states": ["false_positive", "stale_obsolete", "duplicate_of_issue", "defer_to_issue"],
        "resolved_by_code_recheck_changed": True,
    },
    "context": {
        "model_token_budget": 180000,
        "diff_summary_tokens": 4000,
        "per_file_patch_tokens": 3000,
        "total_patch_tokens": 24000,
        "findings_tokens": 12000,
        "openspec_tokens": 8000,
        "memory_tokens": 6000,
    },
    "lifecycle": {
        "max_threads_per_triage": 16,
        "max_root_cause_groups_per_run": 8,
        "char_budget": 24000,
        "terminal_states": ["resolved_by_code", "defer_to_issue", "duplicate_of_issue", "false_positive", "stale_obsolete"],
        "non_terminal_states": ["fix_now", "current_head_keep_open", "needs_human", "blocked_by_conflict"],
    },
    "memory": {
        "enabled": True,
        "root": ".omo/review-memory",
        "notepad_files": ["learnings.md", "decisions.md", "issues.md", "problems.md"],
        "ledger_file": "ledger.json",
        "max_entries": 200,
        "per_file_char_budget": 6000,
        "total_char_budget": 24000,
        "compaction_keep_recent_rounds": 3,
        "provenance_required_for_suppression": True,
        "hmac_env": "CODEX_MEMORY_HMAC_KEY",
    },
    "design": {"require_design_chief": True, "max_clusters": 12, "max_cluster_analysis_batch_size": 4},
    "autofix": {
        "enabled": False,
        "max_rounds": 5,
        "oscillation_window": 10,
        "pingpong_threshold": 2,
        "revert_threshold": 0.8,
        "memory_write_prefix": ".omo/review-memory/",
        "allowed_prefixes": ["src/", "tests/", "docs/"],
        "forbidden_prefixes": [".git/", ".github/workflows/", "setup/codex-review/prompts/", "setup/codex-review/schemas/"],
        "forbidden_files": [],
        "dangerous_keywords": ["secret", "private key", "api key", "nonce", "signature", "signing", "public api"],
    },
    "loop": {"dispatch_per_correlation_cap": 20},
    "tests": {"default_commands": []},
}


def _deep_merge(base: dict[str, Any], override: dict[str, Any]) -> dict[str, Any]:
    out = dict(base)
    for key, value in override.items():
        if isinstance(value, dict) and isinstance(out.get(key), dict):
            out[key] = _deep_merge(out[key], value)
        else:
            out[key] = value
    return out


def _load_yaml_mapping(path: Path, *, label: str) -> dict[str, Any]:
    raw = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    if not isinstance(raw, dict):
        raise ValidationError(f"{label} must contain a YAML object: {path}")
    return raw


def load_config(
    path: str | Path | None = None,
    *,
    override: dict[str, Any] | None = None,
    override_path: str | Path | None = None,
) -> dict[str, Any]:
    """Load the trusted-core config and layer consumer overrides on top.

    Merge precedence (lowest to highest):
      generic DEFAULT_CONFIG -> base config.yml -> override file -> override mapping

    The trusted core ships generic safe defaults; a consumer repo supplies its
    repo-specific values (trusted authors, forbidden files, dangerous keywords,
    test commands) as an override that is deep-merged over those defaults.

    Override sources:
      override         programmatic mapping, highest precedence.
      override_path    explicit override file; strict, must exist if given.
      CODEX_REVIEW_CONFIG_OVERRIDE  ambient override file; best-effort, a
                       missing path is treated as "no override" so the package
                       loads with generic defaults instead of crashing.

    A missing base config.yml at the default location also falls back to the
    generic defaults, so consumers are not required to ship the file. An
    explicit ``path`` that does not exist remains an error.
    """
    p = Path(path) if path else setup_root() / "config.yml"
    if p.exists():
        base = _load_yaml_mapping(p, label="config file")
    elif path is not None:
        raise ValidationError(f"missing config file: {p}")
    else:
        base = {}
    config = _deep_merge(DEFAULT_CONFIG, base)

    if override_path is not None:
        op = Path(override_path)
        if not op.exists():
            raise ValidationError(f"missing config override file: {op}")
        config = _deep_merge(config, _load_yaml_mapping(op, label="config override"))
    else:
        env_override = os.environ.get(CONFIG_OVERRIDE_ENV)
        if env_override:
            ep = Path(env_override)
            if ep.exists():
                config = _deep_merge(config, _load_yaml_mapping(ep, label="config override"))

    if override is not None:
        if not isinstance(override, dict):
            raise ValidationError("config override must be a mapping")
        config = _deep_merge(config, override)

    validate_config(config)
    return config


def get_review_axes(config: dict[str, Any]) -> list[str]:
    axes = list(config.get("review", {}).get("axes", []))
    if not axes:
        raise ValidationError("review.axes must not be empty")
    return axes


def get_lifecycle_policy(config: dict[str, Any]) -> dict[str, Any]:
    return dict(config.get("lifecycle", {}))


def get_autofix_policy(config: dict[str, Any]) -> dict[str, Any]:
    return dict(config.get("autofix", {}))


def get_context_budget(config: dict[str, Any]) -> dict[str, Any]:
    return dict(config.get("context", DEFAULT_CONFIG["context"]))


def validate_config(config: dict[str, Any]) -> None:
    for section in ["trusted", "review", "lifecycle", "memory", "design", "autofix", "loop", "tests"]:
        if section not in config or not isinstance(config[section], dict):
            raise ValidationError(f"config section {section!r} is required")
    axes = config["review"].get("axes")
    if not isinstance(axes, list) or not axes or len(set(axes)) != len(axes):
        raise ValidationError("review.axes must be a non-empty list with unique values")
    auto = config["autofix"]
    for int_key in ["max_rounds", "oscillation_window", "pingpong_threshold"]:
        if int(auto.get(int_key, 0)) < 0:
            raise ValidationError(f"autofix.{int_key} must be non-negative")
    loop = config["loop"]
    if int(loop.get("dispatch_per_correlation_cap", 0)) <= 0:
        raise ValidationError("loop.dispatch_per_correlation_cap must be positive")
    if not (0.0 <= float(auto.get("revert_threshold", 0.8)) <= 1.0):
        raise ValidationError("autofix.revert_threshold must be between 0 and 1")
    forbidden_prefixes = set(auto.get("forbidden_prefixes", []))
    for prefix in auto.get("allowed_prefixes", []):
        if prefix in forbidden_prefixes:
            raise ValidationError(f"prefix cannot be both allowed and forbidden: {prefix}")
    ctx = config["context"] if "context" in config else None
    if ctx is not None:
        if not isinstance(ctx, dict):
            raise ValidationError("config section 'context' must be a mapping")
        for ctx_key in [
            "model_token_budget",
            "diff_summary_tokens",
            "per_file_patch_tokens",
            "total_patch_tokens",
            "findings_tokens",
            "openspec_tokens",
            "memory_tokens",
        ]:
            if ctx_key in ctx and int(ctx[ctx_key]) < 0:
                raise ValidationError(f"context.{ctx_key} must be non-negative")
    memory = config["memory"]
    required_memory_keys = [
        "enabled",
        "root",
        "notepad_files",
        "ledger_file",
        "max_entries",
        "per_file_char_budget",
        "total_char_budget",
        "compaction_keep_recent_rounds",
        "provenance_required_for_suppression",
        "hmac_env",
    ]
    for memory_key in required_memory_keys:
        if memory_key not in memory:
            raise ValidationError(f"memory.{memory_key} is required")
    if not isinstance(memory["enabled"], bool):
        raise ValidationError("memory.enabled must be a boolean")
    if not isinstance(memory["provenance_required_for_suppression"], bool):
        raise ValidationError("memory.provenance_required_for_suppression must be a boolean")
    for str_key in ["root", "ledger_file", "hmac_env"]:
        if not isinstance(memory[str_key], str) or not memory[str_key]:
            raise ValidationError(f"memory.{str_key} must be a non-empty string")
    if not isinstance(memory["notepad_files"], list) or not all(isinstance(item, str) and item for item in memory["notepad_files"]):
        raise ValidationError("memory.notepad_files must be a list of non-empty strings")
    for positive_key in ["max_entries", "per_file_char_budget", "total_char_budget"]:
        if int(memory[positive_key]) <= 0:
            raise ValidationError(f"memory.{positive_key} must be positive")
    if int(memory["compaction_keep_recent_rounds"]) < 0:
        raise ValidationError("memory.compaction_keep_recent_rounds must be non-negative")
    if int(memory["total_char_budget"]) > int(config["lifecycle"].get("char_budget", 0)):
        raise ValidationError("memory.total_char_budget must not exceed lifecycle.char_budget")
