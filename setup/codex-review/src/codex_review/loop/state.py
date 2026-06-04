"""Loop state loaded from payloads and emitted as artifact JSON."""
from __future__ import annotations

import hashlib
import json
import re
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from codex_review.context.diff import parse_unified_diff
from codex_review.core.artifacts import write_json
from codex_review.core.errors import ValidationError

_WS = re.compile(r"\s+")


def empty_loop_state() -> dict[str, Any]:
    return {"schema_version": "loop-state.v1", "recent_pushes": [], "round_count": 0, "dispatch_ledger": []}


def deterministic_state_artifact_name(correlation_id: str, iteration: int | str) -> str:
    safe_correlation = re.sub(r"[^A-Za-z0-9_.-]+", "-", str(correlation_id or "").strip()).strip(".-")
    if not safe_correlation:
        raise ValidationError("correlation_id is required for state artifact name")
    try:
        iteration_number = int(iteration)
    except (TypeError, ValueError):
        raise ValidationError("iteration must be an integer for state artifact name") from None
    if iteration_number < 0:
        raise ValidationError("iteration must be non-negative for state artifact name")
    return f"codex-loop-state-{safe_correlation}-{iteration_number}.json"


def validate_state_artifact_reference(payload: dict[str, Any], *, allow_initial_empty: bool = False) -> dict[str, str]:
    state_run_id = str(payload.get("state_run_id") or "").strip()
    state_artifact_name = str(payload.get("state_artifact_name") or "").strip()
    if allow_initial_empty and not state_run_id and not state_artifact_name:
        try:
            iteration_number = int(payload.get("iteration", 0) or 0)
        except (TypeError, ValueError):
            iteration_number = -1
        if iteration_number == 0:
            return {"state_run_id": "", "state_artifact_name": ""}
    if not state_run_id:
        raise ValidationError("state_run_id is required")
    if not state_run_id.isdigit() or int(state_run_id) <= 0:
        raise ValidationError("state_run_id must be a positive integer string")
    if not state_artifact_name:
        raise ValidationError("state_artifact_name is required")
    if Path(state_artifact_name).name != state_artifact_name or state_artifact_name in {".", ".."}:
        raise ValidationError("state_artifact_name must be a file name, not a path")
    return {"state_run_id": state_run_id, "state_artifact_name": state_artifact_name}


def build_state_artifact_download_command(reference: dict[str, Any], *, output_dir: str | Path) -> list[str]:
    ref = validate_state_artifact_reference(reference)
    return [
        "gh",
        "run",
        "download",
        ref["state_run_id"],
        "--name",
        ref["state_artifact_name"],
        "--dir",
        str(output_dir),
    ]


def state_artifact_pointer_payload(*, state_run_id: int | str, state_artifact_name: str) -> dict[str, str]:
    return validate_state_artifact_reference(
        {"state_run_id": str(state_run_id), "state_artifact_name": state_artifact_name}
    )

def _normalize_loop_state(state: dict[str, Any] | None) -> dict[str, Any]:
    if not state:
        return empty_loop_state()
    out = {**empty_loop_state(), **state}
    out["schema_version"] = "loop-state.v1"
    out["recent_pushes"] = list(out.get("recent_pushes") or [])
    out["round_count"] = int(out.get("round_count", 0) or 0)
    out["dispatch_ledger"] = normalize_dispatch_ledger(out.get("dispatch_ledger"))
    return out


def read_loop_state_payload(payload: dict[str, Any] | None) -> dict[str, Any]:
    if not payload:
        return empty_loop_state()
    if payload.get("schema_version") == "reentry-loop-state.v1":
        loop_state = payload.get("loop_state")
        if loop_state is not None and not isinstance(loop_state, dict):
            raise ValidationError("reentry loop_state must be an object")
        return _normalize_loop_state(loop_state)
    if "loop_state" in payload and isinstance(payload.get("loop_state"), dict):
        return _normalize_loop_state(payload["loop_state"])
    return _normalize_loop_state(payload)


def read_loop_state_artifact(path: str | Path | None) -> dict[str, Any]:
    if not path:
        return empty_loop_state()
    p = Path(path)
    if not p.exists():
        return empty_loop_state()
    try:
        payload = json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValidationError(f"malformed loop-state artifact {path}: {exc}") from exc
    if not isinstance(payload, dict):
        raise ValidationError("loop-state artifact must contain a JSON object")
    return read_loop_state_payload(payload)


def write_loop_state_artifact(state: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, _normalize_loop_state(state), "loop-state.v1")


DISPATCH_LEDGER_SCHEMA = "dispatch-ledger.v1"
DISPATCH_LEDGER_DEFAULT_CAP = 20


def empty_dispatch_ledger() -> dict[str, Any]:
    return {"schema_version": DISPATCH_LEDGER_SCHEMA, "entries": []}


def normalize_dispatch_ledger(ledger: dict[str, Any] | list[dict[str, Any]] | None) -> list[dict[str, Any]]:
    if ledger is None:
        return []
    if isinstance(ledger, dict):
        entries = ledger.get("entries", [])
    else:
        entries = ledger
    if not isinstance(entries, list):
        raise ValidationError("dispatch ledger entries must be a list")
    out: list[dict[str, Any]] = []
    for raw in entries:
        if not isinstance(raw, dict):
            raise ValidationError("dispatch ledger entry must be an object")
        out.append(
            {
                "correlation_id": str(raw.get("correlation_id") or ""),
                "stage": str(raw.get("stage") or raw.get("next_stage") or ""),
                "iteration": int(raw.get("iteration", 0) or 0),
                "head_sha": str(raw.get("head_sha") or ""),
                "state_run_id": str(raw.get("state_run_id") or ""),
                "state_artifact_name": str(raw.get("state_artifact_name") or ""),
                "status": str(raw.get("status") or "emitted"),
            }
        )
    return out


def read_dispatch_ledger_artifact(path: str | Path | None) -> dict[str, Any]:
    if not path:
        return empty_dispatch_ledger()
    p = Path(path)
    if not p.exists():
        return empty_dispatch_ledger()
    try:
        payload = json.loads(p.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValidationError(f"malformed dispatch ledger artifact {path}: {exc}") from exc
    if isinstance(payload, list):
        entries = normalize_dispatch_ledger(payload)
    elif isinstance(payload, dict) and "dispatch_ledger" in payload and "entries" not in payload:
        entries = normalize_dispatch_ledger(payload.get("dispatch_ledger"))
    elif isinstance(payload, dict):
        entries = normalize_dispatch_ledger(payload)
    else:
        raise ValidationError("dispatch ledger artifact must contain a JSON object or list")
    return {"schema_version": DISPATCH_LEDGER_SCHEMA, "entries": entries}


def write_dispatch_ledger_artifact(ledger: dict[str, Any] | list[dict[str, Any]] | None, out_path: str | Path) -> Path:
    return write_json(out_path, {"entries": normalize_dispatch_ledger(ledger)}, DISPATCH_LEDGER_SCHEMA)


def build_dispatch_ledger_entry(payload: dict[str, Any]) -> dict[str, Any]:
    stage = str(payload.get("stage") or payload.get("next_stage") or "")
    return {
        "correlation_id": str(payload.get("correlation_id") or ""),
        "stage": stage,
        "iteration": int(payload.get("iteration", 0) or 0),
        "head_sha": str(payload.get("head_sha") or ""),
        "state_run_id": str(payload.get("state_run_id") or ""),
        "state_artifact_name": str(payload.get("state_artifact_name") or ""),
        "status": str(payload.get("status") or "emitted"),
    }


def dispatch_ledger_key(entry: dict[str, Any]) -> tuple[str, str, int, str]:
    normalized = build_dispatch_ledger_entry(entry)
    return (
        normalized["correlation_id"],
        normalized["stage"],
        normalized["iteration"],
        normalized["head_sha"],
    )


def append_dispatch_ledger_entry(
    ledger: dict[str, Any] | list[dict[str, Any]] | None,
    entry: dict[str, Any],
    *,
    window: int | None = None,
) -> dict[str, Any]:
    entries = normalize_dispatch_ledger(ledger)
    next_entries = [*entries, build_dispatch_ledger_entry(entry)]
    if window is not None and int(window) > 0 and len(next_entries) > int(window):
        next_entries = next_entries[-int(window):]
    return {"schema_version": DISPATCH_LEDGER_SCHEMA, "entries": next_entries}


def dispatch_cap_from_config(config: dict[str, Any] | None) -> int:
    loop = (config or {}).get("loop", {}) or {}
    autofix = (config or {}).get("autofix", {}) or {}
    raw = loop.get("dispatch_per_correlation_cap", autofix.get("max_rounds", DISPATCH_LEDGER_DEFAULT_CAP))
    cap = int(raw)
    return cap if cap > 0 else DISPATCH_LEDGER_DEFAULT_CAP


def evaluate_dispatch_ledger(
    ledger: dict[str, Any] | list[dict[str, Any]] | None,
    entry: dict[str, Any],
    *,
    max_iterations: int,
    dispatch_cap: int,
) -> dict[str, Any]:
    candidate = build_dispatch_ledger_entry(entry)
    entries = normalize_dispatch_ledger(ledger)
    if candidate["iteration"] >= int(max_iterations):
        return {"ok": False, "terminal_reason": "max_iterations", "status": "max_iterations", "entry": candidate}
    matching_emitted = [
        item for item in entries
        if dispatch_ledger_key(item) == dispatch_ledger_key(candidate) and item.get("status") != "staged"
    ]
    if matching_emitted:
        return {"ok": False, "terminal_reason": "dispatch_duplicate", "status": "dispatch_duplicate", "entry": candidate}
    same_correlation = [
        item for item in entries
        if item.get("correlation_id") == candidate["correlation_id"] and item.get("status") != "staged"
    ]
    if len(same_correlation) >= int(dispatch_cap):
        return {"ok": False, "terminal_reason": "max_iterations", "status": "max_iterations", "entry": candidate}
    repeated_signature = [
        item for item in same_correlation
        if item.get("stage") == candidate["stage"] and item.get("head_sha") == candidate["head_sha"]
    ]
    if len(repeated_signature) > 1:
        return {"ok": False, "terminal_reason": "oscillation_detected", "status": "oscillation_detected", "entry": candidate}
    return {"ok": True, "terminal_reason": "", "status": "ok", "entry": candidate}


def build_loop_state(stage: str, decision: dict[str, Any], head_sha: str, artifacts: dict[str, Any]) -> dict[str, Any]:
    material=json.dumps(artifacts or {}, sort_keys=True, default=str).encode("utf-8")
    return {"schema_version": "loop-state.v1", "stage": stage, "decision": decision, "head_sha": head_sha, "artifact_hash": hashlib.sha256(material).hexdigest(), "updated_at": datetime.now(timezone.utc).isoformat()}


def validate_loop_state(state: dict[str, Any], current_pr: dict[str, Any]) -> None:
    current=((current_pr.get("head") or {}).get("sha") or current_pr.get("head_sha"))
    if state.get("head_sha") and current and state["head_sha"] != current:
        raise ValidationError("loop state head_sha does not match current PR head")


# --- Oscillation / round-cap detection (case A) ---------------------------------
#
# The loop persists bounded push history through explicit loop-state artifacts.
# Detection is similarity/identity based, NOT byte-equality: an AI re-fixing the same
# issue produces a byte-different patch every round, so an exact patch hash alone is
# useless for catching A->B->A oscillation.


def _normalize_line(text: str) -> str:
    return _WS.sub(" ", (text or "").strip())


def _line_fingerprint(text: str) -> str:
    return hashlib.sha256(_normalize_line(text).encode("utf-8")).hexdigest()[:16]


def fingerprint_patch(patch_text: str) -> dict[str, Any]:
    """Whitespace-normalized fingerprints of a patch's changed lines + touched paths.

    Robust to cosmetic drift (whitespace/comment wording/ordering) so two rounds that
    add/remove the *same* lines compare equal even when the raw bytes differ.
    """
    files = parse_unified_diff(patch_text or "")
    added: set[str] = set()
    removed: set[str] = set()
    touched: set[str] = set()
    for f in files:
        path = f.get("new_path") or f.get("old_path")
        if path and path != "/dev/null":
            touched.add(str(path))
        for hunk in f.get("hunks", []):
            for line in hunk.get("lines", []):
                norm = _normalize_line(line.get("text"))
                if not norm:
                    continue
                if line.get("kind") == "add":
                    added.add(_line_fingerprint(norm))
                elif line.get("kind") == "del":
                    removed.add(_line_fingerprint(norm))
    return {"added_line_fp": sorted(added), "removed_line_fp": sorted(removed), "touched_paths": sorted(touched)}


def normalized_finding_keys_from_plan(design_plan: dict[str, Any] | None) -> list[str]:
    """Stable-ish identity of WHAT a fix targets, from the design plan edit sequence."""
    keys: set[str] = set()
    for step in (design_plan or {}).get("edit_sequence", []) or []:
        for fid in step.get("finding_ids", []) or []:
            if fid:
                keys.add(str(fid).strip().lower())
        if step.get("task_id"):
            keys.add(str(step["task_id"]).strip().lower())
    return sorted(keys)


def build_push_entry(round_no: int, push_result: dict[str, Any], patch_text: str, design_plan: dict[str, Any] | None) -> dict[str, Any]:
    # Empty patch_sha256 stays falsy so a missing/empty patch can never be mistaken for
    # an "identical patch repeated" match (sha256("") is otherwise a constant).
    patch_sha256 = hashlib.sha256(patch_text.encode("utf-8")).hexdigest() if patch_text else ""
    return {
        "round": int(round_no),
        "commit_sha": push_result.get("commit_sha"),
        "head_sha": push_result.get("head_sha") or push_result.get("old_head"),
        "patch_sha256": patch_sha256,
        "normalized_finding_keys": normalized_finding_keys_from_plan(design_plan),
        **fingerprint_patch(patch_text),
    }


def append_push_to_loop_state(prior: dict[str, Any] | None, entry: dict[str, Any], window: int) -> dict[str, Any]:
    prior = prior or {}
    recent = list(prior.get("recent_pushes", []) or [])
    recent.append(entry)
    if window and len(recent) > int(window):
        recent = recent[-int(window):]
    return {
        **prior,
        "schema_version": "loop-state.v1",
        "stage": "reentry",
        "recent_pushes": recent,
        "round_count": int(prior.get("round_count", 0)) + 1,
        "head_sha": entry.get("head_sha") or prior.get("head_sha"),
        "updated_at": datetime.now(timezone.utc).isoformat(),
    }


def _jaccard(a: list[str] | None, b: list[str] | None) -> float:
    sa, sb = set(a or []), set(b or [])
    if not sa or not sb:
        return 0.0
    return len(sa & sb) / len(sa | sb)


def detect_oscillation(prior: dict[str, Any] | None, candidate: dict[str, Any], config: dict[str, Any] | None) -> dict[str, Any]:
    """Decide whether pushing ``candidate`` would continue a non-converging loop.

    Returns {"ok": bool, "status": str, "reason": str}. ok=False blocks the push so the
    loop escalates to an issue + human instead of ping-ponging forever.

    These convergence guards (round cap + oscillation/pingpong/revert detection) are
    the *only* bound on the autofix loop. Blast-radius caps (max_files/patch_bytes/
    commits/tasks) were intentionally removed because they escalated legitimate large
    fixes to a human; a fix that genuinely converges should be allowed to land however
    big it is, and one that does not is caught here.
    """
    auto = (config or {}).get("autofix", {}) or {}
    max_rounds = int(auto.get("max_rounds", 5))
    pingpong_threshold = int(auto.get("pingpong_threshold", 2))
    revert_threshold = float(auto.get("revert_threshold", 0.8))
    prior = prior or {}
    recent = prior.get("recent_pushes", []) or []
    round_count = int(prior.get("round_count", 0))

    if round_count >= max_rounds:
        return {"ok": False, "status": "max_rounds_reached", "reason": f"autofix rounds {round_count} reached max_rounds {max_rounds}"}
    if candidate.get("patch_sha256") and any(p.get("patch_sha256") == candidate["patch_sha256"] for p in recent):
        return {"ok": False, "status": "oscillation_detected", "reason": "identical merged patch was already pushed in a prior round"}
    for key in candidate.get("normalized_finding_keys", []) or []:
        hits = sum(1 for p in recent if key in (p.get("normalized_finding_keys") or []))
        if hits >= pingpong_threshold:
            return {"ok": False, "status": "oscillation_detected", "reason": f"finding '{key}' was already addressed in {hits} prior round(s) (ping-pong)"}
    cand_added = candidate.get("added_line_fp", []) or []
    cand_paths = set(candidate.get("touched_paths", []) or [])
    for p in recent:
        if cand_paths & set(p.get("touched_paths", []) or []) and _jaccard(cand_added, p.get("removed_line_fp")) >= revert_threshold:
            return {"ok": False, "status": "oscillation_detected", "reason": "patch re-adds lines a prior round removed in the same files (revert loop)"}
    return {"ok": True, "status": "ok", "reason": ""}
