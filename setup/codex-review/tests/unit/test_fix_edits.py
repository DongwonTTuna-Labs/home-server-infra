import subprocess

import pytest

from codex_review.core.errors import ValidationError
from codex_review.patches.fix_edits import apply_edits_and_generate_patch, ensure_patch_from_edits


def _git(args, cwd):
    subprocess.run(["git", *args], cwd=cwd, check=True, capture_output=True)


def _init_repo(path, files):
    _git(["init", "-q"], path)
    _git(["config", "user.email", "t@t"], path)
    _git(["config", "user.name", "t"], path)
    for name, content in files.items():
        (path / name).write_text(content, encoding="utf-8")
    _git(["add", "-A"], path)
    _git(["commit", "-qm", "init"], path)


def _applies(patch, repo):
    proc = subprocess.run(["git", "apply", "--check", "-"], input=patch, text=True, cwd=repo, capture_output=True)
    return proc.returncode == 0


def test_single_edit_generates_applicable_patch(tmp_path):
    _init_repo(tmp_path, {"a.py": "alpha\nbeta\ngamma\n"})
    patch = apply_edits_and_generate_patch([{"path": "a.py", "old_str": "beta", "new_str": "BETA"}], tmp_path)
    assert "a/a.py" in patch and "+BETA" in patch
    assert _applies(patch, tmp_path)


def test_multiple_edits_distinct_files(tmp_path):
    _init_repo(tmp_path, {"a.py": "x = 1\n", "b.py": "y = 2\n"})
    patch = apply_edits_and_generate_patch(
        [{"path": "a.py", "old_str": "x = 1", "new_str": "x = 11"},
         {"path": "b.py", "old_str": "y = 2", "new_str": "y = 22"}],
        tmp_path,
    )
    assert _applies(patch, tmp_path)


def test_new_file_via_empty_old_str(tmp_path):
    _init_repo(tmp_path, {"a.py": "keep\n"})
    patch = apply_edits_and_generate_patch([{"path": "new/created.py", "old_str": "", "new_str": "hello\n"}], tmp_path)
    assert "new file mode" in patch
    assert _applies(patch, tmp_path)


def test_old_str_not_found_raises(tmp_path):
    _init_repo(tmp_path, {"a.py": "alpha\n"})
    with pytest.raises(ValidationError, match="not found"):
        apply_edits_and_generate_patch([{"path": "a.py", "old_str": "missing", "new_str": "x"}], tmp_path)


def test_old_str_not_unique_raises(tmp_path):
    _init_repo(tmp_path, {"a.py": "dup\ndup\n"})
    with pytest.raises(ValidationError, match="not unique"):
        apply_edits_and_generate_patch([{"path": "a.py", "old_str": "dup", "new_str": "x"}], tmp_path)


def test_ensure_patch_from_edits_injects_patch(tmp_path):
    _init_repo(tmp_path, {"a.py": "one\n"})
    obj = {"schema_version": "fix-merge-merged-fix.v1", "edits": [{"path": "a.py", "old_str": "one", "new_str": "ONE"}]}
    out = ensure_patch_from_edits(obj, tmp_path)
    assert out["patch"] and out["patch"] == out["patch_text"]
    assert _applies(out["patch"], tmp_path)


def test_ensure_patch_noop_without_edits(tmp_path):
    obj = {"schema_version": "fix-merge-merged-fix.v1", "patch": "EXISTING"}
    out = ensure_patch_from_edits(obj, tmp_path)
    assert out["patch"] == "EXISTING"


def test_validate_agent_result_materializes_patch_from_edits(tmp_path):
    from codex_review.stages.fix_dispatch.validate_agent_result import validate_fix_agent_result

    _init_repo(tmp_path, {"a.py": "value = 1\n"})
    result = {"task_id": "fix-1", "status": "patched",
              "edits": [{"path": "a.py", "old_str": "value = 1", "new_str": "value = 2"}]}
    task = {"task_id": "fix-1", "allowed_files": ["a.py"]}
    out = validate_fix_agent_result(result, task, {}, tmp_path)
    assert out["patch"] and _applies(out["patch"], tmp_path)
    assert out["policy_report"]


def test_deletion_generates_applicable_patch(tmp_path):
    _init_repo(tmp_path, {"keep.py": "stay\n", "gone.md": "delete me\n"})
    patch = apply_edits_and_generate_patch([], tmp_path, deletions=["gone.md"])
    assert "deleted file mode" in patch and "gone.md" in patch
    assert _applies(patch, tmp_path)


def test_edits_and_deletions_combined(tmp_path):
    _init_repo(tmp_path, {"a.py": "x = 1\n", "old.md": "bye\n"})
    patch = apply_edits_and_generate_patch(
        [{"path": "a.py", "old_str": "x = 1", "new_str": "x = 2"}],
        tmp_path,
        deletions=["old.md"],
    )
    assert "deleted file mode" in patch and "+x = 2" in patch
    assert _applies(patch, tmp_path)


def test_deletion_of_absent_path_is_noop(tmp_path):
    _init_repo(tmp_path, {"a.py": "x\n"})
    patch = apply_edits_and_generate_patch([], tmp_path, deletions=["nope.md"])
    assert patch == ""


def test_ensure_patch_from_deletions_only(tmp_path):
    _init_repo(tmp_path, {"doc.md": "content\n"})
    obj = {"schema_version": "fix-dispatch-agent-result.v1", "status": "patched", "deletions": ["doc.md"]}
    out = ensure_patch_from_edits(obj, tmp_path)
    assert "deleted file mode" in out["patch"]
    assert _applies(out["patch"], tmp_path)


def test_fix_prompt_contracts_describe_edits_not_diff():
    from codex_review.stages.fix_dispatch.prompt import include_patch_output_contract
    from codex_review.stages.fix_merge.prompt import include_final_patch_contract

    for contract in (include_patch_output_contract(""), include_final_patch_contract("")):
        assert "edits" in contract
        assert "old_str" in contract and "new_str" in contract
        assert "deletions" in contract
        assert "unified diff" not in contract.lower().replace("not a unified diff", "")
