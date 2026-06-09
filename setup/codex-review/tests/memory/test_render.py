from __future__ import annotations

from copy import deepcopy
from typing import TypeAlias

from codex_review.memory.render import compact, render_projection_files
from codex_review.memory.types import CATEGORY_NOTEPAD_FILES, SCHEMA_VERSION, validate_review_memory_ledger

Entry: TypeAlias = dict[str, object]
Ledger: TypeAlias = dict[str, object]
MemoryConfig: TypeAlias = dict[str, object]

SCOPE: dict[str, object] = {"repository": "owner/repo", "pr_number": 7, "base_ref": "main"}


def _ledger(entries: list[Entry]) -> Ledger:
    return {"schema_version": SCHEMA_VERSION, "scope": dict(SCOPE), "entries": entries}


def _entry(
    entry_id: str,
    *,
    round: int,
    category: str = "learnings",
    kind: str = "learning",
    body: Entry | None = None,
    trusted: bool = False,
    finding_fingerprint: str | None = None,
    source_stage: str = "review",
) -> Entry:
    entry: Entry = {
        "entry_id": entry_id,
        "created_at": f"2026-06-08T00:00:{round:02d}Z",
        "round": round,
        "head_sha": f"head-{entry_id}",
        "kind": kind,
        "category": category,
        "body": body if body is not None else {"summary": f"summary for {entry_id}"},
        "source_stage": source_stage,
        "trusted": trusted,
    }
    if finding_fingerprint is not None:
        entry["finding_fingerprint"] = finding_fingerprint
    return entry


def _memory_config(**overrides: object) -> MemoryConfig:
    config: MemoryConfig = {
        "max_entries": 200,
        "per_file_char_budget": 6000,
        "total_char_budget": 24000,
        "compaction_keep_recent_rounds": 3,
    }
    config.update(overrides)
    return config


def test_render_projection_files_group_by_category_and_newest_first_with_metadata() -> None:
    secret = "ghp_" + "Aa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8Ii9Jj0KkLlMm"
    ledger = _ledger(
        [
            _entry("learn-old", round=1, body={"summary": "old learning"}),
            _entry(
                "decision-new",
                round=4,
                category="decisions",
                kind="decision",
                body={"summary": "ship the deterministic renderer"},
                trusted=True,
                source_stage="techlead",
            ),
            _entry("issue", round=3, category="issues", kind="open_risk", body={"summary": "risk remains"}),
            _entry(
                "problem",
                round=2,
                category="problems",
                kind="rejected_approach",
                body={"summary": "do not treat markdown as source"},
            ),
            _entry(
                "learn-new",
                round=5,
                body={"summary": f"new learning saw token {secret}"},
                trusted=True,
                finding_fingerprint="finding:abc123",
                source_stage="fix_merge",
            ),
        ]
    )

    md_files = render_projection_files(ledger)

    assert list(md_files) == list(CATEGORY_NOTEPAD_FILES.values())
    learnings = md_files["learnings.md"]
    assert learnings.index("## learn-new") < learnings.index("## learn-old")
    assert "## decision-new" in md_files["decisions.md"]
    assert "## issue" in md_files["issues.md"]
    assert "## problem" in md_files["problems.md"]
    assert "## decision-new" not in learnings
    assert "- kind: `learning`" in learnings
    assert "- round: 5" in learnings
    assert "- head_sha: `head-learn-new`" in learnings
    assert "- source_stage: `fix_merge`" in learnings
    assert "- status: trusted" in learnings
    assert "- fingerprint: `finding:abc123`" in learnings
    assert secret not in learnings
    assert "[REDACTED_SECRET]" in learnings
    assert "Generated from `review-memory.v1` ledger" in learnings


def test_render_projection_files_redacts_secret_like_metadata_values() -> None:
    metadata_secrets = {
        "entry_id": "github_pat_entrySecretAa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8",
        "head_sha": "github_pat_headSecretAa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8",
        "source_stage": "github_pat_stageSecretAa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8",
        "finding_fingerprint": "github_pat_fingerprintSecretAa1Bb2Cc3Dd4Ee5Ff6Gg7Hh8",
    }
    ledger = _ledger(
        [
            {
                **_entry(
                    metadata_secrets["entry_id"],
                    round=6,
                    body={"summary": "metadata redaction regression"},
                    finding_fingerprint=metadata_secrets["finding_fingerprint"],
                    source_stage=metadata_secrets["source_stage"],
                ),
                "head_sha": metadata_secrets["head_sha"],
            }
        ]
    )

    rendered = render_projection_files(ledger)["learnings.md"]

    for raw_secret in metadata_secrets.values():
        assert raw_secret not in rendered
    assert rendered.count("[REDACTED_SECRET]") >= len(metadata_secrets)
    assert "metadata redaction regression" in rendered


def test_compact_summarizes_old_entries_and_respects_render_budgets() -> None:
    entries = [
        _entry(f"learn-{round_number}", round=round_number, body={"summary": "ordinary prose " * 80})
        for round_number in range(1, 7)
    ]
    ledger = _ledger(entries)

    compacted, md_files = compact(
        ledger,
        _memory_config(max_entries=4, per_file_char_budget=1200, total_char_budget=4600, compaction_keep_recent_rounds=2),
    )

    validate_review_memory_ledger(compacted)
    entry_ids = [entry["entry_id"] for entry in compacted["entries"]]
    assert entry_ids[-2:] == ["learn-5", "learn-6"]
    assert entry_ids[0].startswith("compacted-learnings-r1-r4-")
    assert compacted["entries"][0]["body"] == {
        "compacted": True,
        "summary": "Compacted 4 older learnings entries from round(s) 1-4.",
        "entry_count": 4,
        "rounds": {"min": 1, "max": 4},
        "kinds": {"learning": 4},
        "categories": {"learnings": 4},
        "source_stages": {"review": 4},
    }
    assert all(len(text) <= 1200 for text in md_files.values())
    assert sum(len(text) for text in md_files.values()) <= 4600
    assert "Compacted 4 older learnings entries" in md_files["learnings.md"]


def test_compact_keeps_recent_rounds_verbatim() -> None:
    ledger = _ledger(
        [
            _entry(f"decision-{round_number}", round=round_number, category="decisions", kind="decision")
            for round_number in range(1, 6)
        ]
    )

    compacted, _md_files = compact(
        ledger,
        _memory_config(max_entries=3, per_file_char_budget=1600, total_char_budget=5000, compaction_keep_recent_rounds=2),
    )

    entries_by_id = {entry["entry_id"]: entry for entry in compacted["entries"]}
    assert entries_by_id["decision-4"]["body"] == {"summary": "summary for decision-4"}
    assert entries_by_id["decision-5"]["body"] == {"summary": "summary for decision-5"}
    assert entries_by_id["decision-4"]["source_stage"] == "review"
    assert entries_by_id["decision-5"]["source_stage"] == "review"
    assert [entry["round"] for entry in compacted["entries"]] == [3, 4, 5]
    assert compacted["entries"][0]["entry_id"].startswith("compacted-decisions-r1-r3-")


def test_compact_preserves_trusted_resolved_findings_even_when_old_and_over_cap() -> None:
    trusted_resolved = _entry(
        "trusted-resolved",
        round=1,
        kind="resolved_finding",
        trusted=True,
        finding_fingerprint="review:security:nonce",
        body={"summary": "resolved finding must survive"},
        source_stage="techlead",
    )
    ledger = _ledger(
        [
            trusted_resolved,
            _entry("old-advisory", round=2, body={"summary": "can be compacted"}),
            _entry("recent", round=5, body={"summary": "recent round is kept"}),
        ]
    )
    original = deepcopy(ledger)

    compacted, md_files = compact(
        ledger,
        _memory_config(max_entries=2, per_file_char_budget=1200, total_char_budget=4600, compaction_keep_recent_rounds=1),
    )

    assert ledger == original
    entry_ids = [entry["entry_id"] for entry in compacted["entries"]]
    assert "trusted-resolved" in entry_ids
    assert "recent" in entry_ids
    preserved = next(entry for entry in compacted["entries"] if entry["entry_id"] == "trusted-resolved")
    assert preserved == trusted_resolved
    assert "- fingerprint: `review:security:nonce`" in md_files["learnings.md"]
    assert "- status: trusted" in md_files["learnings.md"]


def test_compact_is_deterministic() -> None:
    ledger = _ledger(
        [
            _entry("issue-1", round=1, category="issues", kind="open_risk", body={"summary": "risk one"}),
            _entry("issue-3", round=3, category="issues", kind="open_risk", body={"summary": "risk three"}),
            _entry("issue-2", round=2, category="issues", kind="open_risk", body={"summary": "risk two"}),
            _entry("issue-4", round=4, category="issues", kind="open_risk", body={"summary": "risk four"}),
        ]
    )
    config = _memory_config(max_entries=3, per_file_char_budget=1400, total_char_budget=4600, compaction_keep_recent_rounds=1)

    first_ledger, first_files = compact(deepcopy(ledger), config)
    second_ledger, second_files = compact(deepcopy(ledger), deepcopy(config))

    assert first_ledger == second_ledger
    assert first_files == second_files
    assert render_projection_files(first_ledger) == render_projection_files(second_ledger)
