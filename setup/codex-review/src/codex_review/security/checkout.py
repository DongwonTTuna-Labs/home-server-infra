"""Checkout boundary guards."""
from __future__ import annotations

import os
from pathlib import Path

from codex_review.core.errors import PolicyViolation


def assert_running_from_trusted_checkout() -> dict[str, str | None]:
    trusted=os.environ.get("CODEX_REVIEW_TRUSTED_CHECKOUT") or os.environ.get("GITHUB_WORKSPACE")
    cwd=str(Path.cwd().resolve())
    if trusted and not cwd.startswith(str(Path(trusted).resolve())):
        raise PolicyViolation(f"helper is not running from trusted checkout: cwd={cwd} trusted={trusted}")
    return {"cwd": cwd, "trusted_checkout": trusted}


def assert_pr_checkout_is_read_only() -> dict[str, str | None]:
    pr=os.environ.get("CODEX_REVIEW_PR_CHECKOUT")
    if pr and os.environ.get("CODEX_REVIEW_WRITE_MODE") == "true" and Path.cwd().resolve().as_posix().startswith(Path(pr).resolve().as_posix()):
        raise PolicyViolation("write-mode command is running inside PR head checkout")
    return {"pr_checkout": pr}


def resolve_base_and_head_paths() -> dict[str, str | None]:
    return {"trusted_checkout": os.environ.get("CODEX_REVIEW_TRUSTED_CHECKOUT") or os.environ.get("GITHUB_WORKSPACE"), "pr_checkout": os.environ.get("CODEX_REVIEW_PR_CHECKOUT")}


def prevent_pr_head_setup_execution(path: str | Path) -> None:
    p=Path(path).resolve()
    pr=os.environ.get("CODEX_REVIEW_PR_CHECKOUT")
    if pr and p.as_posix().startswith((Path(pr).resolve() / "setup" / "codex-review").as_posix()):
        raise PolicyViolation("refusing to execute setup/codex-review from PR head checkout")
