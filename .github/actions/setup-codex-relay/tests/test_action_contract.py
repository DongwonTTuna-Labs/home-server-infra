from pathlib import Path

import yaml


ACTION_PATH = Path(__file__).resolve().parents[1] / "action.yml"


def test_setup_action_uses_trusted_actors_csv_with_legacy_input() -> None:
    action = yaml.safe_load(ACTION_PATH.read_text(encoding="utf-8"))
    inputs = action["inputs"]
    run_script = "\n".join(step.get("run", "") for step in action["runs"]["steps"])

    assert "trusted-actors" in inputs
    assert inputs["trusted-actors"]["default"] == "DongwonTTuna"
    assert "trusted-actor" in inputs
    assert inputs["trusted-actor"]["default"] == ""
    assert "TRUSTED_ACTORS" in run_script
    assert "TRUSTED_ACTOR" in run_script
    assert "${{ inputs.trusted-actor }}" in ACTION_PATH.read_text(encoding="utf-8")


def test_setup_action_allows_read_only_shell_without_network_or_secret_leak() -> None:
    action_text = ACTION_PATH.read_text(encoding="utf-8")

    assert '"--disable","shell_tool"' not in action_text
    assert '"sandbox_workspace_write.network_access=false"' in action_text
    assert '"shell_environment_policy.exclude=[\\"AI_RELAY_API_KEY\\"]"' in action_text
