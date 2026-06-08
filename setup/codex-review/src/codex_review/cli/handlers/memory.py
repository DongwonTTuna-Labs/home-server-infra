"""CLI handler: memory commands."""
from __future__ import annotations

import argparse
from typing import Any

from codex_review.memory.merge_guard import assert_review_memory_not_on_base


def handle_memory(args: argparse.Namespace) -> tuple[Any, str | None]:
    cmd = args.command
    if cmd == "assert-not-on-base":
        return assert_review_memory_not_on_base(args.repo_path, args.base_ref or args.base or "main"), None
    raise ValueError(f"unknown memory command: {cmd}")
