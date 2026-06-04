"""Shared helpers for Codex loop workflow-shape tests.

The Codex review pipeline was migrated from a multi-file, label-driven split
(``codex-review.yml`` -> ``codex-design.yml`` -> ``codex-fix.yml`` ->
``codex-issue.yml`` plus a ``setup-codex-review`` composite action) into a
single reusable core workflow (``codex-loop-reusable.yml``) that is driven by a
``stage`` input and invoked by thin adapters (``codex-loop-dispatch.yml``,
``codex-loop-manual.yml``).

These helpers aggregate across whichever pipeline files currently exist so the
shape/security invariants can be asserted against the live topology while still
allowing per-file structural assertions.
"""
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS_DIR = ROOT / ".github" / "workflows"

# The reusable core runs the model via the pinned-by-tag native codex-action and
# relays every request through the self-hosted codex-lb responses proxy.
CODEX_ACTION = "openai/codex-action@v1"
RESPONSES_ENDPOINT = "https://relay-ai.dongwontuna.net/v1/responses"
OIDC_MINT_COMMAND = "codex-review oidc relay-token"

# Files that together implement the Codex loop pipeline, in invocation order:
# the reusable core plus the dispatch/manual adapters that call it.
PIPELINE_FILENAMES = [
    "codex-loop-reusable.yml",
    "codex-loop-dispatch.yml",
    "codex-loop-manual.yml",
]

REUSABLE = WORKFLOWS_DIR / "codex-loop-reusable.yml"
DISPATCH = WORKFLOWS_DIR / "codex-loop-dispatch.yml"
MANUAL = WORKFLOWS_DIR / "codex-loop-manual.yml"


def workflow_path(name: str) -> Path:
    return WORKFLOWS_DIR / name


def exists(name: str) -> bool:
    return workflow_path(name).exists()


def existing_pipeline_files() -> list[Path]:
    return [workflow_path(n) for n in PIPELINE_FILENAMES if exists(n)]


def load(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def jobs_of(name: str) -> dict:
    return load(workflow_path(name)).get("jobs") or {}


def reusable_jobs() -> dict:
    return jobs_of("codex-loop-reusable.yml")


def all_jobs() -> dict:
    """Merge jobs across every existing pipeline file (job names are unique)."""
    merged: dict = {}
    for path in existing_pipeline_files():
        for job_name, job in (load(path).get("jobs") or {}).items():
            merged[job_name] = job
    return merged


def all_text() -> str:
    return "\n".join(p.read_text(encoding="utf-8") for p in existing_pipeline_files())


def iter_all_steps():
    for path in existing_pipeline_files():
        for job_name, job in (load(path).get("jobs") or {}).items():
            for step in job.get("steps", []) or []:
                yield job_name, step


def codex_action_steps():
    return [(n, s) for n, s in iter_all_steps() if s.get("uses") == CODEX_ACTION]
