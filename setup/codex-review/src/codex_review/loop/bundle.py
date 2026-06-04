"""State-bundle transport contract: required files per bundle kind + integrity verify.

Each Codex loop stage uploads its hand-off state as a deterministically named
artifact (a *state bundle*). The next stage downloads exactly that artifact by
explicit ``state_run_id`` + ``state_artifact_name`` (never head-SHA / latest-run
discovery) and must be able to trust that the bundle carries the files it needs.

This module is the single source of truth for *what files each bundle kind must
contain*. ``verify_bundle`` turns that contract into an integrity verdict so the
workflow can route a missing or corrupt bundle to the canonical terminal reasons
``artifact_missing`` / ``artifact_schema_invalid`` instead of silently treating an
empty download as success.

The bundle carries only sanitized, validated state (decisions, plans, manifests,
publish reports, route verdicts, loop history pointers). Raw model output and
secrets are intentionally NOT part of any contract.

``dispatch-ledger.json`` is required in continuation-capable state bundles. It is
artifact state only: duplicate/cap decisions are made from this file, not labels,
comments, issues, branches, or latest-run discovery.
"""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any

# Canonical terminal_reason values (see schemas/terminal-reason.v1.json).
ARTIFACT_MISSING = "artifact_missing"
ARTIFACT_SCHEMA_INVALID = "artifact_schema_invalid"

BUNDLE_VERIFY_SCHEMA = "loop-state-bundle-verify.v1"

# Files carried in every bundle that can lead to another repository_dispatch.
DISPATCH_LEDGER_FILE = "dispatch-ledger.json"
DEFERRED_CARRY_FORWARD: tuple[str, ...] = ()

# Per-kind state-bundle contract. ``artifact_prefix`` matches the deterministic
# upload name the producing stage uses; ``required_files`` are the files the next
# stage depends on and that must exist + parse as JSON.
BUNDLE_CONTRACTS: dict[str, dict[str, Any]] = {
    "review": {
        "artifact_prefix": "codex-loop-review-state-",
        "required_files": (
            "techlead-decision.json",
            "review-publication.json",
            "publish-report.json",
            "route.json",
            DISPATCH_LEDGER_FILE,
        ),
        "deferred_files": (),
    },
    "design": {
        "artifact_prefix": "codex-loop-design-state-",
        "required_files": (
            "design-plan.json",
            "chief-decision.json",
            "publish-report.json",
            "route.json",
            DISPATCH_LEDGER_FILE,
        ),
        "deferred_files": (),
    },
    "fix": {
        "artifact_prefix": "codex-loop-fix-",
        "required_files": (
            "manifest.json",
            "merged-fix.json",
            "validated-fix.json",
            "semantic-safety.json",
            DISPATCH_LEDGER_FILE,
        ),
        "deferred_files": (),
    },
    "issue": {
        "artifact_prefix": "codex-loop-issue-fallback-",
        "required_files": (
            "reason.json",
            "plan.json",
            "issue-fallback.json",
        ),
        "deferred_files": (),
    },
    "loop-state": {
        # The generic per-run loop-state JSON artifact (setup-relay upload). It is
        # a single-file bundle, not a directory of stage outputs.
        "artifact_prefix": "codex-loop-state-",
        "required_files": (".codex-loop-state.json",),
        "deferred_files": (),
    },
}

SUPPORTED_BUNDLE_KINDS = tuple(BUNDLE_CONTRACTS.keys())


def infer_bundle_kind(artifact_name: str | None) -> str:
    """Map a state artifact name to its bundle kind by deterministic prefix."""
    name = (artifact_name or "").strip()
    if name:
        for kind, spec in BUNDLE_CONTRACTS.items():
            prefix = spec["artifact_prefix"]
            if prefix and name.startswith(prefix):
                return kind
    return "unknown"


def required_files_for(kind: str) -> tuple[str, ...]:
    spec = BUNDLE_CONTRACTS.get(kind)
    return tuple(spec["required_files"]) if spec else ()


def _is_valid_json_file(path: Path) -> bool:
    try:
        json.loads(path.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError, ValueError):
        return False
    return True


def verify_bundle(
    directory: str | Path,
    kind: str = "auto",
    *,
    artifact_name: str | None = None,
    allow_initial_empty: bool = False,
) -> dict[str, Any]:
    """Verify a downloaded state bundle against its required-file contract.

    Returns a verdict dict with ``ok`` and a canonical ``terminal_reason``:

    - A required file that is absent yields ``ok=False`` and
      ``terminal_reason="artifact_missing"`` (missing beats invalid).
    - A required file that does not parse as JSON yields ``ok=False`` and
      ``terminal_reason="artifact_schema_invalid"``.
    - When ``allow_initial_empty`` is set and the bundle is *entirely* empty
      (directory absent or no required file present), the verdict is ``ok=True``
      with ``terminal_reason=""`` and ``bootstrap=True``. This is the only path
      where a missing bundle is acceptable, reserved for the very first loop
      iteration that has no upstream state yet. A *partially* present bundle is
      always treated as corrupt and never bootstrapped.
    """
    resolved_kind = kind
    if kind in (None, "", "auto"):
        resolved_kind = infer_bundle_kind(artifact_name)

    base = Path(directory)
    spec = BUNDLE_CONTRACTS.get(resolved_kind)
    if spec is None:
        # An unrecognized bundle kind cannot be asserted against any contract.
        # Treat it as missing so an unknown/empty download never passes as success.
        return {
            "schema_version": BUNDLE_VERIFY_SCHEMA,
            "ok": False,
            "kind": resolved_kind,
            "artifact_name": artifact_name or "",
            "directory": str(base),
            "required_files": [],
            "deferred_files": list(DEFERRED_CARRY_FORWARD),
            "present": [],
            "missing": [],
            "invalid": [],
            "bootstrap": False,
            "terminal_reason": ARTIFACT_MISSING,
            "detail": f"unknown bundle kind for artifact_name={artifact_name!r}",
        }

    required = tuple(spec["required_files"])
    present: list[str] = []
    missing: list[str] = []
    invalid: list[str] = []

    for fname in required:
        fpath = base / fname
        if not fpath.is_file():
            missing.append(fname)
        elif not _is_valid_json_file(fpath):
            invalid.append(fname)
        else:
            present.append(fname)

    bootstrap = False
    if allow_initial_empty and not present and not invalid:
        # Entirely empty bundle (nothing usable) at the very first iteration.
        bootstrap = True
        missing = []

    if missing:
        ok = False
        terminal_reason = ARTIFACT_MISSING
    elif invalid:
        ok = False
        terminal_reason = ARTIFACT_SCHEMA_INVALID
    else:
        ok = True
        terminal_reason = ""

    return {
        "schema_version": BUNDLE_VERIFY_SCHEMA,
        "ok": ok,
        "kind": resolved_kind,
        "artifact_name": artifact_name or "",
        "directory": str(base),
        "required_files": list(required),
        "deferred_files": list(spec["deferred_files"]),
        "present": present,
        "missing": missing,
        "invalid": invalid,
        "bootstrap": bootstrap,
        "terminal_reason": terminal_reason,
    }
