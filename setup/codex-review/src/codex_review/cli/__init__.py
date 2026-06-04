"""CLI dispatcher for Codex Review v3 helpers."""
from __future__ import annotations

import argparse
import json
import sys

from codex_review.core.config import load_config
from codex_review.core.errors import format_error
from codex_review.core.output import append_step_summary
from codex_review.cli._helpers import _add_common, _emit, _preferred_artifact_paths  # noqa: F401  (re-exported for tests)
from codex_review.cli.handlers.event import handle_event as _handle_event  # noqa: F401  (re-exported for tests)
from codex_review.cli.registry import AREAS, HANDLERS, NEEDS_CONFIG


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="codex-review")
    parser.add_argument("area", choices=AREAS)
    _add_common(parser)
    args = parser.parse_args(argv)
    try:
        handler = HANDLERS[args.area]
        if args.area in NEEDS_CONFIG:
            payload, schema = handler(args, load_config(args.config, override_path=args.config_override))
        else:
            payload, schema = handler(args)
        _emit(payload, args.out, schema)
        if args.summary and isinstance(payload, str):
            append_step_summary(payload)
        return 0
    except Exception as exc:
        print(json.dumps(format_error(exc, {"area": getattr(args, "area", None), "command": getattr(args, "command", None)}), ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
