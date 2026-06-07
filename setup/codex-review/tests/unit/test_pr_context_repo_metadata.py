"""owner/repo must be populated even without a GitHub event payload.

In the repository_dispatch loop there is no `pull_request` event, so
build_pr_context seeds "owner" as None. A regression where
include_repository_metadata used setdefault left owner=None, which made the
push stage fail with "autofix push requires owner/repo/pr_number".
"""
from codex_review.context.pr import build_pr_context, include_repository_metadata


def test_owner_backfilled_from_fetched_pr_without_event():
    pr = {
        "number": 98,
        "head": {"ref": "test/codex-push-smoke", "sha": "a04e"},
        "base": {"ref": "main", "repo": {"full_name": "DongwonTTuna-Labs/rs-builder-relayer-client"}},
    }
    ctx = build_pr_context({}, pr, [], "", {})
    assert ctx["owner"] == "DongwonTTuna-Labs"
    assert ctx["repo"] == "rs-builder-relayer-client"
    assert ctx["pr_number"] == 98
    assert ctx["head_ref"] == "test/codex-push-smoke"


def test_include_repository_metadata_overwrites_none_owner():
    ctx = include_repository_metadata({"repository": "acme/widgets", "owner": None})
    assert ctx["owner"] == "acme"
    assert ctx["repo"] == "widgets"


def test_include_repository_metadata_keeps_existing_truthy_owner():
    ctx = include_repository_metadata({"repository": "acme/widgets", "owner": "octo", "repo": "kit"})
    assert ctx["owner"] == "octo"
    assert ctx["repo"] == "kit"
