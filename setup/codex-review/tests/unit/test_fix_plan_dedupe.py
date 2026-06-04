from codex_review.stages.fix_dispatch.plan import plan_fix_tasks


def test_plan_dedupes_duplicate_model_task_ids():
    # The design model emitted two steps with the same task_id touching
    # different files (so they don't merge). plan must not hard-fail; it
    # disambiguates the collision deterministically.
    design_plan = {
        "edit_sequence": [
            {"task_id": "dup", "files": ["a.py"], "summary": "fix a"},
            {"task_id": "dup", "files": ["b.py"], "summary": "fix b"},
        ],
    }
    chief = {"status": "approved_for_fix", "fix_policy": {"max_tasks": 4}}
    manifest = plan_fix_tasks(design_plan, chief, {})
    ids = [t["task_id"] for t in manifest["tasks"]]
    assert ids == ["dup", "dup-2"], ids
    assert len(ids) == len(set(ids))


def test_plan_keeps_distinct_ids_untouched():
    design_plan = {
        "edit_sequence": [
            {"task_id": "one", "files": ["a.py"], "summary": "fix a"},
            {"task_id": "two", "files": ["b.py"], "summary": "fix b"},
        ],
    }
    chief = {"status": "approved_for_fix", "fix_policy": {"max_tasks": 4}}
    manifest = plan_fix_tasks(design_plan, chief, {})
    assert [t["task_id"] for t in manifest["tasks"]] == ["one", "two"]


def test_plan_allows_more_than_four_tasks():
    # The max_tasks cap was removed: a design with many distinct-file tasks must
    # plan all of them instead of escalating to an issue.
    design_plan = {
        "edit_sequence": [
            {"task_id": f"fix-{i}", "files": [f"src/f{i}.py"], "summary": f"fix {i}"}
            for i in range(6)
        ],
    }
    chief = {"status": "approved_for_fix", "fix_policy": {"allowed_prefixes": ["src/"]}}
    manifest = plan_fix_tasks(design_plan, chief, {})
    assert len(manifest["tasks"]) == 6
    assert not manifest.get("no_fix_needed")


def test_plan_approved_with_no_tasks_is_no_fix_needed():
    # An approved design with nothing concrete to change is a no-op (LGTM),
    # not a blocker that escalates to an issue.
    design_plan = {"edit_sequence": []}
    chief = {"status": "approved_for_fix", "fix_policy": {"allowed_prefixes": ["src/"]}}
    manifest = plan_fix_tasks(design_plan, chief, {})
    assert manifest["tasks"] == []
    assert manifest["no_fix_needed"] is True
