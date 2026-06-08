from __future__ import annotations

import json

from codex_review.cli import main
from codex_review.context import openspec
from codex_review.context.diff import build_changed_line_map, parse_unified_diff, serialize_changed_line_map
from codex_review.context.pr import build_pr_context

CODE_PATH = "src/app.py"
OPENSPEC_PATH = "openspec/changes/demo/tasks.md"
MEMORY_PATHS = [
    ".omo/review-memory/pr-7/ledger.json",
    ".omo/review-memory/pr-7/learnings.md",
    ".omo/review-memory/pr-7/nested/round-1/scratch.json",
]


def _patch(added: str = "new") -> str:
    return f"@@ -1 +1,2 @@\n old\n+{added}"


def _file(path: str, *, added: str = "new", status: str = "modified") -> dict[str, object]:
    return {"filename": path, "status": status, "additions": 1, "deletions": 0, "patch": _patch(added)}


def _diff(paths: list[str]) -> str:
    parts = []
    for index, path in enumerate(paths, start=1):
        parts.append(f"diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n{_patch(f'new {index}')}")
    return "\n".join(parts)


def _pr(files: list[dict[str, object]]) -> dict[str, object]:
    return build_pr_context(
        {"repository": {"full_name": "owner/repo", "owner": {"login": "owner"}}, "number": 7},
        {"number": 7, "title": "Task 10", "body": "", "head": {}, "base": {}},
        files,
        _diff([str(file["filename"]) for file in files]),
        {},
    )


def test_pr_context_excludes_review_memory_changed_files_and_keeps_code_path() -> None:
    files = [_file(CODE_PATH), *[_file(path, added=path) for path in MEMORY_PATHS]]

    context = _pr(files)

    assert [file["filename"] for file in context["changed_files"]] == [CODE_PATH]
    assert [item["filename"] for item in context["changed_files_summary"]] == [CODE_PATH]
    assert context["changed_line_map"] == {CODE_PATH: [2]}
    assert CODE_PATH in context["diff_summary"]
    for path in MEMORY_PATHS:
        assert path not in {file["filename"] for file in context["changed_files"]}
        assert path not in context["changed_line_map"]
        assert path not in context["diff_summary"]


def test_memory_only_pr_context_yields_empty_changed_code_set() -> None:
    context = _pr([_file(path, added=path) for path in MEMORY_PATHS])

    assert context["changed_files"] == []
    assert context["changed_files_summary"] == []
    assert context["changed_line_map"] == {}
    assert context["diff_summary"] == ""


def test_changed_lines_cli_excludes_review_memory_paths_and_keeps_code_path(tmp_path) -> None:
    payload_path = tmp_path / "files.json"
    output_path = tmp_path / "changed-lines.json"
    payload_path.write_text(json.dumps({"files": [_file(CODE_PATH), *[_file(path, added=path) for path in MEMORY_PATHS]]}), encoding="utf-8")

    assert main(["context", "changed-lines", "--in", str(payload_path), "--out", str(output_path)]) == 0

    payload = json.loads(output_path.read_text(encoding="utf-8"))
    assert payload["changed_line_map"] == {CODE_PATH: [2]}


def test_changed_line_builder_excludes_memory_only_diff_string() -> None:
    changed = build_changed_line_map(_diff(MEMORY_PATHS))

    assert serialize_changed_line_map(changed) == {}


def test_openspec_changed_file_discovery_excludes_memory_paths_and_keeps_openspec_path() -> None:
    pr_context = {
        "changed_files_summary": [
            {"filename": OPENSPEC_PATH},
            *[{"filename": path} for path in MEMORY_PATHS],
        ]
    }

    sources = openspec._sources_from_changed_files(pr_context)

    assert sources == [{"type": "path", "path": OPENSPEC_PATH}]


def test_openspec_changed_file_discovery_memory_only_is_empty() -> None:
    sources = openspec._sources_from_changed_files({"changed_files_summary": [{"filename": path} for path in MEMORY_PATHS]})

    assert sources == []


def test_collect_openspec_context_excludes_memory_paths_from_linked_github_pr(monkeypatch, tmp_path) -> None:
    def fake_get_pull_request(owner, repo, pr_number, token):
        return {"title": "Linked OpenSpec", "body": "", "head": {"sha": "head-sha"}}

    def fake_list_pull_request_files(owner, repo, pr_number, token):
        return [
            {"filename": ".omo/review-memory/pr-7/ledger.json"},
            {"filename": "openspec/changes/linked/tasks.md"},
        ]

    def fake_read_github_documents(source, token, budget):
        return [
            {
                "source_type": source.get("type"),
                "owner": source.get("owner"),
                "repo": source.get("repo"),
                "path": source.get("path"),
                "ref": source.get("ref"),
                "sha256": "sha",
                "content": "- [ ] linked",
                "truncated": False,
            }
        ]

    monkeypatch.setattr(openspec, "get_pull_request", fake_get_pull_request)
    monkeypatch.setattr(openspec, "list_pull_request_files", fake_list_pull_request_files)
    monkeypatch.setattr(openspec, "_read_github_documents", fake_read_github_documents)

    context = openspec.collect_openspec_context(
        {"owner": "owner", "repo": "repo", "title": "", "body": "See https://github.com/owner/repo/pull/7"},
        repo_path=tmp_path,
        token="token",
    )

    github_file_paths = [source.get("path") for source in context["sources"] if source.get("type") == "github_file"]
    assert github_file_paths == ["openspec/changes/linked/tasks.md"]
    assert [document["path"] for document in context["documents"]] == ["openspec/changes/linked/tasks.md"]
    assert all(path is None or ".omo/review-memory" not in path for path in github_file_paths)


def test_raw_unified_diff_parser_keeps_memory_entries_for_parser_semantics() -> None:
    paths = [CODE_PATH, *MEMORY_PATHS]
    parsed = parse_unified_diff(_diff(paths))

    assert [file["new_path"] for file in parsed] == paths
