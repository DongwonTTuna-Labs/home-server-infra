from __future__ import annotations

import json

from codex_review.context.openspec import collect_openspec_context, extract_openspec_sources, render_openspec_context_markdown
from codex_review.cli import main


def _write(path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_extracts_openspec_sources_from_pr_text():
    text = """
    Implements openspec/changes/deposit-wallet/tasks.md.
    Spec PR: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/pull/12
    Design: https://github.com/DongwonTTuna-Labs/rs-builder-relayer-client/blob/feature/x/openspec/changes/deposit-wallet/design.md
    """

    sources = extract_openspec_sources(
        text,
        owner="DongwonTTuna-Labs",
        repo="rs-builder-relayer-client",
    )

    kinds = {source["type"] for source in sources}
    assert {"path", "github_pr", "github_file"}.issubset(kinds)
    assert any(source.get("path") == "openspec/changes/deposit-wallet/tasks.md" for source in sources)
    assert any(source.get("path") == "openspec/changes/deposit-wallet/design.md" for source in sources)



def test_extracts_external_fission_ai_openspec_sources():
    text = """
    Implement according to https://github.com/Fission-AI/OpenSpec/pull/41
    and https://github.com/Fission-AI/OpenSpec/tree/main/openspec/changes/codex-review-lgtm-loop-smoke
    """

    sources = extract_openspec_sources(text, owner="DongwonTTuna-Labs", repo="rs-builder-relayer-client")

    assert any(source.get("type") == "github_pr" and source.get("owner") == "Fission-AI" and source.get("repo") == "OpenSpec" for source in sources)
    tree_source = next(source for source in sources if source.get("type") == "github_file" and source.get("owner") == "Fission-AI")
    assert tree_source["path"] == "openspec/changes/codex-review-lgtm-loop-smoke"
    assert tree_source["ref"] == "main"

def test_collects_local_openspec_change_documents(tmp_path):
    _write(tmp_path / "openspec/changes/deposit-wallet/proposal.md", "# Proposal\nShip deposit wallet")
    _write(tmp_path / "openspec/changes/deposit-wallet/design.md", "# Design\nUse official SDK parity")
    _write(tmp_path / "openspec/changes/deposit-wallet/tasks.md", "- [ ] Implement")
    _write(tmp_path / "openspec/changes/deposit-wallet/specs/deposit/spec.md", "## ADDED Requirements\n- MUST serialize WALLET")
    _write(tmp_path / "openspec/config.yaml", "project: rs-builder-relayer-client\n")

    pr_context = {
        "owner": "DongwonTTuna-Labs",
        "repo": "rs-builder-relayer-client",
        "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
        "pr_number": 41,
        "title": "Implement deposit wallet",
        "body": "See openspec/changes/deposit-wallet",
        "changed_files_summary": [],
    }

    context = collect_openspec_context(pr_context, repo_path=tmp_path)

    assert context["present"] is True
    assert context["status"] == "ready"
    paths = {doc["path"] for doc in context["documents"]}
    assert "openspec/changes/deposit-wallet/proposal.md" in paths
    assert "openspec/changes/deposit-wallet/design.md" in paths
    assert "openspec/changes/deposit-wallet/tasks.md" in paths
    assert "openspec/changes/deposit-wallet/specs/deposit/spec.md" in paths
    assert "openspec/config.yaml" in paths
    rendered = render_openspec_context_markdown(context)
    assert "OpenSpec context" in rendered
    assert "Ship deposit wallet" in rendered


def test_missing_openspec_context_routes_to_missing_spec_decision(tmp_path):
    context = collect_openspec_context(
        {
            "owner": "DongwonTTuna-Labs",
            "repo": "rs-builder-relayer-client",
            "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
            "pr_number": 41,
            "title": "Implement something",
            "body": "No spec link here",
        },
        repo_path=tmp_path,
    )

    assert context["present"] is False
    assert context["status"] == "missing_openspec_spec"
    assert context["decision"] == "missing_openspec_spec"


def test_openspec_context_cli_writes_json_and_markdown(tmp_path):
    _write(tmp_path / "openspec/changes/demo/tasks.md", "- [ ] Do it")
    pr_context_path = tmp_path / "pr-context.json"
    context_path = tmp_path / "openspec-context.json"
    markdown_path = tmp_path / "openspec-context.md"
    pr_context_path.write_text(
        json.dumps(
            {
                "owner": "DongwonTTuna-Labs",
                "repo": "rs-builder-relayer-client",
                "repository": "DongwonTTuna-Labs/rs-builder-relayer-client",
                "pr_number": 41,
                "title": "Demo",
                "body": "openspec/changes/demo/tasks.md",
            }
        ),
        encoding="utf-8",
    )

    assert main(["context", "openspec", "--pr-context", str(pr_context_path), "--repo-path", str(tmp_path), "--out", str(context_path)]) == 0
    assert main(["context", "openspec-markdown", "--in", str(context_path), "--out", str(markdown_path)]) == 0

    assert json.loads(context_path.read_text(encoding="utf-8"))["present"] is True
    assert "OpenSpec context" in markdown_path.read_text(encoding="utf-8")
