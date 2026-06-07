"""The live-readiness gate is a single named invariant, not inlined everywhere.

The Codex loop runs live (model calls, PR comments, autofix push, continuation
dispatch) for eligible PRs; there is no dry-run flag. To keep that safe and
readable, the gating predicate lives in exactly two named outputs:

  trust-and-stale-guard.outputs.eligible   = trusted && !fork && !stale
  setup-relay.outputs.live_ready           = valid && eligible && relay_configured

Every live model step gates on `live_ready`; nothing re-spells the trust triple
inside a step `if:`. These tests pin that structure so it can't silently drift
back into duplicated guards or resurrect the removed flags.
"""
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[4]
WORKFLOWS = ROOT / ".github" / "workflows"
CORE = WORKFLOWS / "codex-loop-reusable.yml"
CALLERS = [CORE, WORKFLOWS / "codex-loop-manual.yml", WORKFLOWS / "codex-loop-dispatch.yml"]
MODEL_ACTION = "openai/codex-action@v1"


def _doc(path: Path) -> dict:
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def _jobs(path: Path) -> dict:
    return _doc(path).get("jobs") or {}


def _all_ifs(path: Path):
    for job_name, job in _jobs(path).items():
        if isinstance(job.get("if"), str):
            yield f"{job_name} (job)", job["if"]
        for step in job.get("steps", []) or []:
            if isinstance(step.get("if"), str):
                yield f"{job_name}/{step.get('name')}", step["if"]


def test_no_dry_run_or_enable_live_autofix_anywhere():
    for path in CALLERS:
        text = path.read_text(encoding="utf-8")
        assert "dry_run" not in text, f"{path.name}: dry_run flag must be gone"
        assert "enable_live_autofix" not in text, f"{path.name}: enable_live_autofix flag must be gone"


def test_named_gate_outputs_are_defined():
    text = CORE.read_text(encoding="utf-8")
    assert "eligible: ${{ steps.guard.outputs.eligible }}" in text
    assert "live_ready: ${{ steps.relay.outputs.live_ready }}" in text


def test_no_if_gate_inlines_the_fork_or_stale_predicate():
    # fork/stale are folded into `eligible`; they must never appear in an if: gate.
    for where, cond in _all_ifs(CORE):
        assert "fork ==" not in cond, f"{where}: inlines fork predicate; use eligible/live_ready"
        assert "stale ==" not in cond, f"{where}: inlines stale predicate; use eligible/live_ready"


def test_every_live_model_step_gates_on_live_ready():
    steps_seen = 0
    for job_name, job in _jobs(CORE).items():
        for step in job.get("steps", []) or []:
            if step.get("uses") == MODEL_ACTION:
                steps_seen += 1
                assert "live_ready" in (step.get("if") or ""), (
                    f"{job_name}/{step.get('name')}: model step must gate on live_ready"
                )
    assert steps_seen >= 9, f"expected >=9 model steps, saw {steps_seen}"
