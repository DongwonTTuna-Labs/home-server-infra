from pathlib import Path
import json

ROOT = Path(__file__).resolve().parents[4]
SCHEMA_DIR = ROOT / "setup" / "codex-review" / "schemas"


def test_all_schema_files_require_schema_version():
    for path in SCHEMA_DIR.glob("*.schema.json"):
        data = json.loads(path.read_text(encoding="utf-8"))
        assert "schema_version" in data.get("required", [])
        assert "schema_version" in data.get("properties", {})


def test_stage_artifact_schemas_are_not_placeholders():
    for path in SCHEMA_DIR.glob("stage*.schema.json"):
        data = json.loads(path.read_text(encoding="utf-8"))
        text = json.dumps(data)
        assert "x_required_implementation_work" not in text
        assert "SPEC" + "-ONLY" not in text
