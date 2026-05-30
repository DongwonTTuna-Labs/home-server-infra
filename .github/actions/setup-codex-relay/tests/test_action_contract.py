from pathlib import Path

import yaml


ACTION_PATH = Path(__file__).resolve().parents[1] / "action.yml"


def test_setup_action_uses_trusted_actors_csv_without_legacy_input() -> None:
    action = yaml.safe_load(ACTION_PATH.read_text(encoding="utf-8"))
    inputs = action["inputs"]
    run_script = "\n".join(step.get("run", "") for step in action["runs"]["steps"])

    assert "trusted-actors" in inputs
    assert inputs["trusted-actors"]["default"] == "DongwonTTuna"
    assert "trusted-actor" not in inputs
    assert "TRUSTED_ACTORS" in run_script
    assert "${{ inputs.trusted-actor }}" not in ACTION_PATH.read_text(encoding="utf-8")
