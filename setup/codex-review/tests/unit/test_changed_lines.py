from codex_review.context.diff import build_changed_line_map, serialize_changed_line_map


def test_build_changed_line_map_extracts_github_file_patch_lines():
    changed = build_changed_line_map(
        [
            {
                "filename": "src/lib.rs",
                "patch": "@@ -8,6 +8,8 @@ fn demo() {\n context\n+added\n+more\n unchanged",
            }
        ]
    )

    assert serialize_changed_line_map(changed) == {"src/lib.rs": [9, 10]}


def test_build_changed_line_map_extracts_each_file_patch_independently():
    changed = build_changed_line_map(
        [
            {"filename": "a.py", "patch": "@@ -1 +1,2 @@\n old\n+new"},
            {"filename": "b.py", "patch": "@@ -20,2 +20,3 @@\n keep\n+added\n keep"},
        ]
    )

    assert serialize_changed_line_map(changed) == {"a.py": [2], "b.py": [21]}
