"""Shared helpers for the Codex Review CLI handlers."""
from __future__ import annotations

import argparse
import glob
import json
import os
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import read_json, read_text, write_json, write_text
from codex_review.core.errors import ValidationError


def _maybe_json(path: str | None, default: Any = None) -> Any:
    if not path:
        return default
    try:
        return json.loads(Path(path).read_text(encoding="utf-8"))
    except FileNotFoundError:
        raise ValidationError(f"missing JSON artifact: {path}") from None
    except json.JSONDecodeError as exc:
        raise ValidationError(f"malformed JSON artifact {path}: {exc}") from exc


def _json_or_default(path: str | None, default: Any) -> Any:
    if not path:
        return default
    p = Path(path)
    if not p.exists():
        return default
    return _maybe_json(path, default)


def _maybe_text(path: str | None, default: str = "") -> str:
    if not path:
        return default
    p = Path(path)
    if not p.exists():
        return default
    return read_text(path)


def _emit(payload: Any, out: str | None = None, schema_version: str | None = None) -> None:
    if out:
        if isinstance(payload, dict):
            write_json(out, payload, schema_version)
        else:
            write_text(out, str(payload))
    else:
        print(json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True) if isinstance(payload, (dict, list)) else str(payload))


def _artifact_paths(values: list[str] | None, *, names: tuple[str, ...] = ("*.json",)) -> list[str]:
    """Expand files, directories and globs into deterministic artifact paths."""
    out: list[str] = []
    for value in values or []:
        matches = glob.glob(value)
        candidates = matches or [value]
        for candidate in candidates:
            p = Path(candidate)
            if p.is_dir():
                for pattern in names:
                    out.extend(str(x) for x in sorted(p.rglob(pattern)))
            elif p.exists():
                out.append(str(p))
    seen: set[str] = set()
    deduped: list[str] = []
    for path in sorted(out):
        if path not in seen:
            seen.add(path)
            deduped.append(path)
    return deduped


def _preferred_artifact_paths(values: list[str] | None, *, primary: str, fallback: str) -> list[str]:
    paths = _artifact_paths(values, names=(primary,))
    return paths or _artifact_paths(values, names=(fallback,))


def _safe_path_component(value: Any) -> str:
    text = str(value or "").strip()
    safe = "".join(ch if ch.isalnum() or ch in {"-", "_", "."} else "_" for ch in text)
    return safe.strip("._") or "task"


def _repo_parts_from_context(ctx: dict[str, Any]) -> tuple[str | None, str | None]:
    owner = ctx.get("owner")
    repo = ctx.get("repo")
    repository = ctx.get("repository") or ctx.get("base_repo_full_name")
    if (not owner or not repo) and isinstance(repository, str) and "/" in repository:
        owner, repo = repository.split("/", 1)
    return owner, repo


def _default_inspection_evidence(purpose: str, observation: str) -> list[dict[str, str]]:
    return [{"path": "AGENTS.md", "purpose": purpose, "observation": observation}]


def _add_common(p: argparse.ArgumentParser) -> None:
    p.add_argument("command", nargs="?")
    p.add_argument("--config", default=None)
    p.add_argument("--config-override", dest="config_override", default=None)
    p.add_argument("--in", dest="in_path", default=None)
    p.add_argument("--out", default=None)
    p.add_argument("--inventory", default=None)
    p.add_argument("--result", default=None)
    p.add_argument("--pr-context", default=None)
    p.add_argument("--review-context", default=None)
    p.add_argument("--docs-context", default=None)
    p.add_argument("--memory-context", default=None)
    p.add_argument("--openspec-context", default=None)
    p.add_argument("--changed-lines", default=None)
    p.add_argument("--axis", default=None)
    p.add_argument("--artifacts", nargs="*", default=None)
    p.add_argument("--token", default=os.environ.get("GITHUB_TOKEN"))
    p.add_argument("--base", default=None)
    p.add_argument("--base-ref", dest="base_ref", default=None)
    p.add_argument("--head", default=None)
    p.add_argument("--repo-path", default=".")
    p.add_argument("--patch", default=None)
    p.add_argument("--dry-run", action="store_true")
    p.add_argument("--summary", action="store_true")
    p.add_argument("--event", default=None)
    p.add_argument("--loop-state", default=None)
    p.add_argument("--ledger", default=None)
    p.add_argument("--chief-decision", default=None)
    p.add_argument("--design-plan", default=None)
    p.add_argument("--mode", default=None)
    p.add_argument("--stage", default=None)
    p.add_argument("--prompt", default=None)
    p.add_argument("--prompt-out", default=None)
    p.add_argument("--raw-out", default=None)
    p.add_argument("--model-command", default=os.environ.get("CODEX_REVIEW_MODEL_COMMAND"))
    p.add_argument("--work-dir", default=None)
    p.add_argument("--model-cwd", default=os.environ.get("CODEX_REVIEW_MODEL_CWD") or os.environ.get("CODEX_REVIEW_TRUSTED_CHECKOUT"))
    p.add_argument("--validation", default=None)
    p.add_argument("--schema", default=None)
    p.add_argument("--semantic-safety", default=None)
    p.add_argument("--audience", default=None)
    p.add_argument("--broker-url", default=None)
    p.add_argument("--name", default=None)
    p.add_argument("--dir", dest="dir", default=None)
    p.add_argument("--kind", default=None)
    p.add_argument("--allow-initial-empty", dest="allow_initial_empty", action="store_true")


def _model_or_fallback(args: argparse.Namespace, *, stage: str, expected_schema: str, fallback: dict[str, Any]) -> dict[str, Any]:
    from codex_review.model.adapter import run_model_or_fallback
    return run_model_or_fallback(
        stage=stage,
        prompt_path=args.prompt,
        output_path=args.out,
        expected_schema=expected_schema,
        fallback=fallback,
        model_command=args.model_command,
        cwd=args.model_cwd or args.repo_path,
        target_repo_path=args.repo_path if args.repo_path not in {None, "."} else None,
    )
