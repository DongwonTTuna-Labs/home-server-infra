"""Shared helpers for Codex loop workflow-shape tests.

The Codex review pipeline now uses a single reusable core workflow
(``codex-loop-reusable.yml``) that runs the full review -> design -> fix -> push
chain in one run, plus thin dispatch/manual adapters that pass only the next
iteration's entry inputs.

These helpers aggregate across the active pipeline files so shape/security
invariants can be asserted against the live topology while still allowing
per-file structural assertions.
"""
from collections.abc import Iterator
from pathlib import Path
from typing import TypeAlias, cast

import yaml

YamlScalar: TypeAlias = str | int | bool | None
YamlValue: TypeAlias = YamlScalar | list["YamlValue"] | dict[str, "YamlValue"]
YamlMap: TypeAlias = dict[str, YamlValue]
YamlStep: TypeAlias = YamlMap
YamlSteps: TypeAlias = list[YamlStep]

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


def _yaml_key(key: object) -> str:
    if key is True:
        return "on"
    assert isinstance(key, str), f"expected YAML string key, got {key!r}"
    return key


def _normalize_yaml(value: object) -> YamlValue:
    if isinstance(value, str) or isinstance(value, bool) or isinstance(value, int) or value is None:
        return value
    if isinstance(value, list):
        return [_normalize_yaml(item) for item in cast(list[object], value)]
    if isinstance(value, dict):
        return {
            _yaml_key(key): _normalize_yaml(item)
            for key, item in cast(dict[object, object], value).items()
        }
    raise AssertionError(f"unsupported YAML value: {value!r}")


def _as_map(value: YamlValue, label: str) -> YamlMap:
    assert isinstance(value, dict), f"expected mapping at {label}"
    return value


def _as_steps(value: YamlValue, label: str) -> YamlSteps:
    assert isinstance(value, list), f"expected step list at {label}"
    steps: YamlSteps = []
    for item in value:
        assert isinstance(item, dict), f"expected step mapping at {label}"
        steps.append(item)
    return steps


def workflow_path(name: str) -> Path:
    return WORKFLOWS_DIR / name


def exists(name: str) -> bool:
    return workflow_path(name).exists()


def existing_pipeline_files() -> list[Path]:
    return [workflow_path(name) for name in PIPELINE_FILENAMES if exists(name)]


def load(path: Path) -> YamlMap:
    loaded = cast(object, yaml.safe_load(path.read_text(encoding="utf-8")))
    return _as_map(_normalize_yaml(loaded), "workflow")


def jobs_of(name: str) -> dict[str, YamlMap]:
    jobs = _as_map(load(workflow_path(name)).get("jobs") or {}, "jobs")
    return {job_name: _as_map(job, f"jobs.{job_name}") for job_name, job in jobs.items()}


def reusable_jobs() -> dict[str, YamlMap]:
    return jobs_of("codex-loop-reusable.yml")


def all_jobs() -> dict[str, YamlMap]:
    """Merge jobs across every existing pipeline file; job names are unique."""
    merged: dict[str, YamlMap] = {}
    for path in existing_pipeline_files():
        jobs = _as_map(load(path).get("jobs") or {}, f"{path.name}.jobs")
        for job_name, job in jobs.items():
            merged[job_name] = _as_map(job, f"{path.name}.jobs.{job_name}")
    return merged


def all_text() -> str:
    return "\n".join(path.read_text(encoding="utf-8") for path in existing_pipeline_files())


def iter_all_steps() -> Iterator[tuple[str, YamlStep]]:
    for path in existing_pipeline_files():
        jobs = _as_map(load(path).get("jobs") or {}, f"{path.name}.jobs")
        for job_name, job in jobs.items():
            step_values = _as_steps(_as_map(job, f"{path.name}.jobs.{job_name}").get("steps") or [], f"{path.name}.{job_name}.steps")
            for step in step_values:
                yield job_name, step


def codex_action_steps() -> list[tuple[str, YamlStep]]:
    return [(name, step) for name, step in iter_all_steps() if step.get("uses") == CODEX_ACTION]
