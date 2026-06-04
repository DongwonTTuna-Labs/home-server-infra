"""Small logging helpers that work in GitHub Actions."""
from __future__ import annotations

from contextlib import contextmanager
import sys


def log_info(message: str) -> None:
    print(f"[codex-review] {message}", file=sys.stderr)


def log_warning(message: str) -> None:
    print(f"::warning::{message}", file=sys.stderr)


def log_error(message: str) -> None:
    print(f"::error::{message}", file=sys.stderr)


@contextmanager
def group(title: str):
    print(f"::group::{title}", file=sys.stderr)
    try:
        yield
    finally:
        print("::endgroup::", file=sys.stderr)
