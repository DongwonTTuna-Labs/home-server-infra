from _pipeline import all_text


def test_no_inline_python_or_schema_bloat():
    # Invariant holds across every pipeline workflow file: helper logic is driven
    # through the installed `codex-review` console entrypoint, never inline python
    # heredocs or inlined JSON-Schema blobs.
    text = all_text()
    assert "python - <<" not in text
    assert "python3 - <<" not in text
    assert "json-schema.org" not in text
    # The helper is installed via the @main setup-codex-review composite action
    # (from the action's own bundled source), then invoked as a console script
    # (not a vendored bin path, not the PR-head tree).
    assert "uses: DongwonTTuna-Labs/home-server-infra/.github/actions/setup-codex-review@" in text
    assert "trusted-core/setup/codex-review" not in text
    assert "bin/codex-review" not in text
    assert "codex-review loop read-state" in text
