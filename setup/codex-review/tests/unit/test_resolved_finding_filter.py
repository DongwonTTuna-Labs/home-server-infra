"""Tests for re-flag suppression: resolved marker, harvest, and filter (PR1/C)."""
from __future__ import annotations

import json
from pathlib import Path

from codex_review.cli import main
from codex_review.github.markers import parse_marker, render_inline_review_marker
from codex_review.stages.resolve_gate.collect import collect_resolved_memory
from codex_review.stages.resolve_gate.render import render_resolve_reply
from codex_review.stages.techlead.filter_resolved import filter_findings_against_resolved
from codex_review.stages.techlead.validate import validate_techlead_decision

CONFIG = str(Path(__file__).resolve().parents[2] / "config.yml")
OVERRIDE = str(Path(__file__).resolve().parents[2] / "tests" / "fixtures" / "config" / "consumer_override.yml")


# ---- C1: resolution reply carries a machine-readable marker ----

def test_render_resolve_reply_embeds_resolved_marker():
    body = render_resolve_reply({"state": "false_positive", "evidence": "not a real bug", "root_cause_key": "K_nonce"}, head_sha="s9")
    assert "not a real bug" in body  # prose preserved
    marker = parse_marker(body, "codex-review:resolved")
    assert marker == {"state": "false_positive", "head_sha": "s9", "root_cause_key": "K_nonce"}


# ---- C2: harvest resolved memory from a resolved thread ----

def _resolved_thread(state, author="codex-bot"):
    return {
        "thread_id": "T1",
        "isResolved": True,
        "path": "src/a.rs",
        "line": 10,
        "comments": [
            {"author": author, "path": "src/a.rs", "line": 10, "body": render_inline_review_marker("F1", "K_nonce") + "\nNonce reused."},
            {"author": author, "body": render_resolve_reply({"state": state, "evidence": "by design", "root_cause_key": "K_nonce"}, head_sha="s9")},
        ],
    }


def test_collect_resolved_memory_recovers_key_and_state():
    mem = collect_resolved_memory([_resolved_thread("false_positive")], {"codex_review_authors": ["codex-bot"]})
    assert len(mem) == 1
    assert mem[0]["root_cause_key"] == "K_nonce"
    assert mem[0]["state"] == "false_positive"
    assert mem[0]["path"] == "src/a.rs" and mem[0]["line"] == 10


def test_collect_resolved_memory_skips_unresolved_and_non_codex():
    open_thread = {**_resolved_thread("false_positive"), "isResolved": False}
    foreign = {"thread_id": "T2", "isResolved": True, "comments": [{"author": "someone", "body": "looks fine"}]}
    assert collect_resolved_memory([open_thread, foreign], {"codex_review_authors": ["codex-bot"]}) == []


# ---- C3: filter findings against resolved memory ----

def _finding(fid, key, file="src/a.rs", line=10, sev="high"):
    return {"schema_version": "shared-review-finding.v1", "finding_id": fid, "root_cause_key": key, "file": file, "line": line, "severity": sev}


def _mem(key, state, path="src/a.rs", line=10):
    return {"items": [{"thread_id": "T1", "root_cause_key": key, "state": state, "reason": "prior reason", "path": path, "line": line}]}


def test_filter_drops_false_positive_reflag():
    combined = {"findings": [_finding("F1", "K_nonce"), _finding("F2", "K_other", file="src/b.rs", line=20)]}
    out, suppressed = filter_findings_against_resolved(combined, _mem("K_nonce", "false_positive"), {}, {})
    assert {f["finding_id"] for f in out["findings"]} == {"F2"}
    assert out["suppressed_resolved_count"] == 1
    assert suppressed[0]["previously_resolved_as"] == "false_positive"


def test_filter_resolved_by_code_dropped_when_file_unchanged():
    combined = {"findings": [_finding("F1", "K_nonce")]}
    out, _ = filter_findings_against_resolved(combined, _mem("K_nonce", "resolved_by_code"), {"src/a.rs": [99]}, {})
    assert out["findings"] == []  # line 10 not in changed lines -> prior resolution stands


def test_filter_resolved_by_code_reallowed_when_location_changed():
    combined = {"findings": [_finding("F1", "K_nonce", line=10)]}
    out, _ = filter_findings_against_resolved(combined, _mem("K_nonce", "resolved_by_code"), {"src/a.rs": [10, 11]}, {})
    assert {f["finding_id"] for f in out["findings"]} == {"F1"}  # re-broken region -> allowed
    assert out["findings"][0]["previously_resolved_as"] == "resolved_by_code"


def test_filter_unknown_state_annotates_not_drops():
    combined = {"findings": [_finding("F1", "K_nonce")]}
    out, suppressed = filter_findings_against_resolved(combined, _mem("K_nonce", None), {}, {})
    assert {f["finding_id"] for f in out["findings"]} == {"F1"} and suppressed == []
    assert out["findings"][0]["previously_resolved_as"] is None


def test_filter_matches_by_path_line_when_key_differs():
    combined = {"findings": [_finding("F1", "DIFFERENT_KEY", file="src/a.rs", line=10)]}
    out, _ = filter_findings_against_resolved(combined, _mem("K_nonce", "false_positive", path="src/a.rs", line=10), {}, {})
    assert out["findings"] == []


def test_filter_no_match_keeps_everything():
    combined = {"findings": [_finding("F1", "K_x", file="src/z.rs", line=5)]}
    out, suppressed = filter_findings_against_resolved(combined, _mem("K_nonce", "false_positive"), {}, {})
    assert len(out["findings"]) == 1 and suppressed == []


def test_filtered_output_satisfies_techlead_coverage_contract():
    # The filtered findings must be a valid contract for the techlead: decisions
    # covering the kept set validate, while a suppressed finding has no decision.
    combined = {"schema_version": "review-combined-findings.v1", "findings": [_finding("F1", "K_nonce"), _finding("F2", "K_other", file="src/b.rs", line=20)]}
    filtered, _ = filter_findings_against_resolved(combined, _mem("K_nonce", "false_positive"), {}, {})
    decision = {
        "schema_version": "techlead-decision.v1",
        "decisions": [{"finding_id": "F2", "action": "publish_only"}],
        "inspection_evidence": [{"path": "AGENTS.md", "purpose": "p", "observation": "o"}],
    }
    validated = validate_techlead_decision(decision, filtered, {"autofix": {}}, repo_path=None)
    assert {d["finding_id"] for d in validated["decisions"]} == {"F2"}


# ---- CLI wiring round-trip ----

def test_cli_collect_resolved_and_filter_roundtrip(tmp_path):
    threads_file = tmp_path / "threads.json"
    threads_file.write_text(json.dumps({"threads": [_resolved_thread("false_positive", author="codex-reviewer-for-dongwonttuna")]}), encoding="utf-8")
    pr_file = tmp_path / "pr.json"
    pr_file.write_text(json.dumps({"head_sha": "s9", "pr_number": 1, "changed_line_map": {}}), encoding="utf-8")
    mem_out = tmp_path / "resolved-memory.json"
    rc = main(["resolve_gate", "collect-resolved", "--config", CONFIG, "--config-override", OVERRIDE, "--in", str(threads_file), "--pr-context", str(pr_file), "--out", str(mem_out)])
    assert rc == 0
    mem = json.loads(mem_out.read_text(encoding="utf-8"))
    assert mem["count"] == 1 and mem["items"][0]["root_cause_key"] == "K_nonce"

    combined_file = tmp_path / "combined.json"
    combined_file.write_text(json.dumps({"schema_version": "review-combined-findings.v1", "findings": [_finding("F1", "K_nonce")]}), encoding="utf-8")
    filtered_out = tmp_path / "filtered.json"
    rc = main(["techlead", "filter-resolved", "--config", CONFIG, "--in", str(combined_file), "--inventory", str(mem_out), "--pr-context", str(pr_file), "--out", str(filtered_out)])
    assert rc == 0
    filtered = json.loads(filtered_out.read_text(encoding="utf-8"))
    assert filtered["findings"] == [] and filtered["suppressed_resolved_count"] == 1
