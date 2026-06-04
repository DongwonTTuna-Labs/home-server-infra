from __future__ import annotations

from codex_review.core.schema import load_schema_json, make_openai_structured_output_schema


ACTION_SCHEMA_NAMES = [
    "resolve-gate-lifecycle-result.v1",
    "review-axis-findings.v1",
    "techlead-decision.v1",
    "design-inventory.v1",
    "design-clusters.v1",
    "design-cluster-analysis.v1",
    "design-plan.v1",
    "design-chief-decision.v1",
    "fix-dispatch-agent-result.v1",
    "fix-merge-merged-fix.v1",
    "fix-merge-semantic-patch-safety.v1",
]


def iter_object_schemas(node, path=()):
    if isinstance(node, dict):
        if node.get("type") == "object" or "properties" in node:
            yield path, node
        for key, value in node.items():
            yield from iter_object_schemas(value, (*path, str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from iter_object_schemas(value, (*path, str(index)))


def iter_array_schemas(node, path=()):
    if isinstance(node, dict):
        if node.get("type") == "array":
            yield path, node
        for key, value in node.items():
            yield from iter_array_schemas(value, (*path, str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from iter_array_schemas(value, (*path, str(index)))


def iter_enum_schemas(node, path=()):
    if isinstance(node, dict):
        if "enum" in node:
            yield path, node
        for key, value in node.items():
            yield from iter_enum_schemas(value, (*path, str(key)))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from iter_enum_schemas(value, (*path, str(index)))


def test_openai_action_schemas_are_strict_structured_outputs():
    for name in ACTION_SCHEMA_NAMES:
        schema = make_openai_structured_output_schema(load_schema_json(name))
        for path, obj in iter_object_schemas(schema):
            assert obj.get("additionalProperties") is False, (name, path)
            properties = obj.get("properties", {})
            assert set(obj.get("required", [])) == set(properties), (name, path)
        for path, arr in iter_array_schemas(schema):
            assert "items" in arr, (name, path)
        for path, enum_schema in iter_enum_schemas(schema):
            assert "type" in enum_schema, (name, path)
            if None in enum_schema.get("enum", []):
                enum_type = enum_schema["type"]
                enum_types = enum_type if isinstance(enum_type, list) else [enum_type]
                assert "null" in enum_types, (name, path)


def test_design_plan_schema_does_not_expose_open_questions():
    schema = load_schema_json("design-plan.v1")
    assert "open_questions" not in schema["properties"]
    assert "acceptance_criteria" in schema["properties"]
    assert "openspec_backed" in schema["properties"]
    assert "execution_blockers" in schema["properties"]

    strict = make_openai_structured_output_schema(schema)
    assert "open_questions" not in strict["properties"]
    assert "acceptance_criteria" in strict["properties"]


def test_openai_strict_schema_keeps_defer_issue_payload_shape():
    schema = make_openai_structured_output_schema(load_schema_json("resolve-gate-lifecycle-result.v1"))
    issue = schema["properties"]["decisions"]["items"]["properties"]["issue_request"]
    assert "null" in issue["type"]
    assert set(issue["properties"]) == {"title", "body", "root_cause_key", "labels"}


def test_openai_strict_schema_keeps_fix_policy_payload_shape():
    schema = make_openai_structured_output_schema(load_schema_json("design-chief-decision.v1"))
    policy = schema["properties"]["fix_policy"]
    assert "null" in policy["type"]
    assert {"allowed_files", "allowed_prefixes", "forbidden_files", "forbidden_prefixes"}.issubset(policy["properties"])


def test_review_to_design_chief_schemas_require_inspection_evidence():
    for name in [
        "review-axis-findings.v1",
        "techlead-decision.v1",
        "design-plan.v1",
        "design-chief-decision.v1",
    ]:
        schema = make_openai_structured_output_schema(load_schema_json(name))
        assert "inspection_evidence" in schema["properties"], name
        assert "inspection_evidence" in schema["required"], name
        evidence = schema["properties"]["inspection_evidence"]
        assert evidence["type"] == "array", name
        item = evidence["items"]
        assert set(item["properties"]) == {"path", "purpose", "observation"}, name
        assert set(item["required"]) == {"path", "purpose", "observation"}, name
        assert item["additionalProperties"] is False, name
