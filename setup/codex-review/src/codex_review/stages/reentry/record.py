"""Record next-run reentry state."""
from __future__ import annotations
from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.loop.state import build_loop_state
from codex_review.memory.writer import write_terminal_memory_commit
from codex_review.security.redaction import safe_log_value


def build_reentry_record(push_result: dict[str, Any], loop_state: dict[str, Any], artifacts: dict[str, Any]) -> dict[str, Any]:
    pushed = bool(push_result.get("pushed"))
    state = loop_state or build_loop_state(
        "reentry",
        {"next_entry": "resolve_gate_on_synchronize" if pushed else "none", "pushed": pushed},
        push_result.get("commit_sha") or "",
        artifacts or {"push_result": push_result},
    )
    return {
        "schema_version": "reentry-loop-state.v1",
        "pushed": pushed,
        "commit_sha": push_result.get("commit_sha"),
        "next_entry": "resolve_gate_on_synchronize" if pushed else "none",
        "loop_state": state,
        "artifacts": artifacts,
        "persisted": False,
    }


def write_reentry_artifact(record: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, record, "reentry-loop-state.v1")


def persist_reentry_loop_state(
    record: dict[str, Any],
    pr_context: dict[str, Any] | None = None,
    token: str | None = None,
    repo_path: str | Path | None = None,
    config: dict[str, Any] | None = None,
) -> dict[str, Any]:
    if not record.get("pushed"):
        out = {**record, "persisted": False, "persist_reason": "no push occurred"}
        try:
            out["memory_write"] = write_terminal_memory_commit(out, pr_context or {}, token, repo_path or Path.cwd(), config)
        except Exception as exc:
            out["memory_write"] = {
                "status": "failed",
                "reason": "terminal memory write failed",
                "non_fatal": True,
                "pushed": False,
                "error_type": exc.__class__.__name__,
                "error": safe_log_value(exc),
            }
        return out
    state = record.get("loop_state") or build_loop_state("reentry", {"next_entry": record.get("next_entry")}, record.get("commit_sha") or "", record.get("artifacts", {}))
    out = dict(record)
    out["loop_state"] = state
    out["persisted"] = True
    out["persist_transport"] = "artifact"
    return out


# Backward-compatible helper retained for older imports.
def update_loop_state_after_push(record: dict[str, Any], token: str | None = None) -> dict[str, Any]:
    return build_loop_state("reentry", {"next_entry": record.get("next_entry")}, record.get("commit_sha") or "", record.get("artifacts", {}))
