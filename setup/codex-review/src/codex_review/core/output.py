"""GitHub Actions output and step summary helpers."""
from __future__ import annotations

import json
import os
import uuid
from pathlib import Path
from typing import Any


def _append(path_env: str, content: str) -> None:
    path = os.environ.get(path_env)
    if path:
        Path(path).parent.mkdir(parents=True, exist_ok=True)
        with open(path, "a", encoding="utf-8") as fh:
            fh.write(content)
    else:
        print(content, end="")



def mask_secret(value: str | None) -> None:
    """Mask a dynamically generated secret in GitHub Actions logs."""
    if value and os.environ.get("GITHUB_ACTIONS"):
        print(f"::add-mask::{value}")


def write_output(name: str, value: str | bool | int | float | None) -> None:
    text = "" if value is None else str(value)
    if "\n" in text:
        delim = f"codex_{uuid.uuid4().hex}"
        _append("GITHUB_OUTPUT", f"{name}<<{delim}\n{text}\n{delim}\n")
    else:
        _append("GITHUB_OUTPUT", f"{name}={text}\n")


def write_json_output(name: str, payload: Any) -> None:
    write_output(name, json.dumps(payload, ensure_ascii=False, sort_keys=True))


def append_step_summary(markdown: str) -> None:
    _append("GITHUB_STEP_SUMMARY", markdown if markdown.endswith("\n") else markdown + "\n")


def set_route_outputs(gate_result: dict[str, Any]) -> None:
    route = gate_result.get("route") or gate_result.get("next_route") or gate_result.get("status") or "stop_noop"
    write_output("route", route)
    write_output("run_review", route == "run_review")
    write_output("run_design", route == "run_design_from_existing_threads")
    write_output("needs_human", route == "stop_needs_human")
    write_output("stop", str(route).startswith("stop_"))
