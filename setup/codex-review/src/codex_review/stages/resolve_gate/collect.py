"""Stage00 thread inventory collection."""
from __future__ import annotations

from pathlib import Path
from typing import Any

from codex_review.core.artifacts import write_json
from codex_review.github.markers import extract_root_cause_metadata, has_marker, parse_marker
from codex_review.github.review_threads import is_thread_resolved, normalize_thread_node
from codex_review.security.provenance import is_trusted_codex_review_author


def _comments(thread: dict[str, Any]) -> list[dict[str, Any]]:
    if isinstance(thread.get("comments"), dict):
        return thread.get("comments", {}).get("nodes", []) or []
    return thread.get("comments") or []


def _author(comment: dict[str, Any]) -> str | None:
    a=comment.get("author")
    return a.get("login") if isinstance(a, dict) else a


def filter_eligible_threads(threads: list[dict[str, Any]], current_head_sha: str | None, policy: dict[str, Any]) -> list[dict[str, Any]]:
    eligible=[]
    for raw in threads or []:
        thread = normalize_thread_node(raw) if "thread_id" not in raw and "comments" in raw else raw
        if is_thread_resolved(thread):
            continue
        comments=_comments(thread)
        codex_comments=[]
        for c in comments:
            body=c.get("body", "")
            if not is_trusted_codex_review_author(_author(c), policy):
                continue
            if not (has_marker(body, "codex-review:inline") or has_marker(body, "codex-review:lifecycle") or "codex-review" in body):
                continue
            codex_comments.append(c)
        if not codex_comments:
            continue
        head_values={c.get("commit_id") or c.get("commit", {}).get("oid") for c in codex_comments if c.get("commit_id") or isinstance(c.get("commit"), dict)}
        is_current_head = bool(current_head_sha and current_head_sha in head_values)
        if is_current_head and not policy.get("include_current_head_threads_for_manual_triage", False):
            continue
        thread=dict(thread)
        thread["eligible_comments"]=codex_comments
        thread["is_current_head"] = is_current_head
        eligible.append(thread)
    return eligible


def build_resolve_item(thread: dict[str, Any], pr_context: dict[str, Any]) -> dict[str, Any]:
    comments=thread.get("eligible_comments") or _comments(thread)
    comment=comments[-1] if comments else {}
    body=comment.get("body", "")
    meta=extract_root_cause_metadata(body)
    item={
        "thread_id": thread.get("thread_id") or thread.get("id"),
        "comment_id": comment.get("id"),
        "path": comment.get("path") or thread.get("path") or thread.get("file"),
        "line": comment.get("line") or thread.get("line"),
        "author": _author(comment),
        "body": body,
        "root_cause_key": meta.get("root_cause_key"),
        "marker": meta.get("marker"),
        "metadata_valid": bool(meta.get("valid")),
        "is_current_head": bool(thread.get("is_current_head")),
        "head_sha": pr_context.get("head_sha"),
        "forced_needs_human": False,
        "forced_reason": None,
    }
    return item


def aggregate_root_cause_metadata(items: list[dict[str, Any]]) -> dict[str, Any]:
    groups: dict[str, list[str]]={}
    invalid=[]
    for item in items:
        key=item.get("root_cause_key")
        if not key or not item.get("metadata_valid"):
            invalid.append(item.get("thread_id"))
            continue
        groups.setdefault(str(key), []).append(str(item.get("thread_id")))
    return {"groups": groups, "invalid_thread_ids": invalid}


def mark_forced_needs_human(items: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen={}
    for item in items:
        reason=None
        if item.get("is_current_head"):
            reason="current_head_comment"
        elif not item.get("thread_id"):
            reason="missing_thread_id"
        elif not item.get("metadata_valid") or not item.get("root_cause_key"):
            reason="invalid_or_missing_codex_metadata"
        if reason:
            item["forced_needs_human"]=True; item["forced_reason"]=reason
        key=item.get("root_cause_key")
        if key:
            seen.setdefault(key, []).append(item)
    for group in seen.values():
        valid={i.get("metadata_valid") for i in group}
        if False in valid and True in valid:
            for i in group:
                i["forced_needs_human"]=True; i["forced_reason"]="conflicting_root_cause_metadata"
    return items


def collect_thread_inventory(pr_context: dict[str, Any], threads: list[dict[str, Any]], comments: list[dict[str, Any]] | None, config: dict[str, Any]) -> dict[str, Any]:
    policy=config.get("trusted", config)
    eligible=filter_eligible_threads(threads, pr_context.get("head_sha"), policy)
    items=[build_resolve_item(t, pr_context) for t in eligible]
    mark_forced_needs_human(items)
    return {"schema_version": "resolve-gate-thread-inventory.v1", "head_sha": pr_context.get("head_sha"), "pr_number": pr_context.get("pr_number"), "items": items, "root_cause_summary": aggregate_root_cause_metadata(items), "thread_count": len(items)}


def write_thread_inventory(inventory: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, inventory, "resolve-gate-thread-inventory.v1")


def collect_resolved_memory(threads: list[dict[str, Any]], policy: dict[str, Any]) -> list[dict[str, Any]]:
    """Harvest ALREADY-resolved Codex threads so later runs can avoid re-flagging them.

    This is the inverse of ``filter_eligible_threads`` (which skips resolved threads).
    For each resolved Codex thread we recover the issue identity (``root_cause_key`` from
    the original inline marker) and the lifecycle ``state`` (from the ``codex-review:resolved``
    marker the resolve reply now embeds), plus the human-readable reason.
    """
    out: list[dict[str, Any]] = []
    for raw in threads or []:
        thread = normalize_thread_node(raw) if "thread_id" not in raw and "comments" in raw else raw
        if not is_thread_resolved(thread):
            continue
        comments = _comments(thread)
        if not any(is_trusted_codex_review_author(_author(c), policy) and "codex-review" in (c.get("body") or "") for c in comments):
            continue
        root_cause_key = path = line = state = reason = head_sha = None
        for c in comments:
            body = c.get("body", "")
            if root_cause_key is None:
                meta = extract_root_cause_metadata(body)
                if meta.get("valid") and meta.get("root_cause_key") and meta.get("marker") in {"codex-review:inline", "codex-review:lifecycle", "text"}:
                    root_cause_key = meta["root_cause_key"]
                    path = c.get("path") or thread.get("path")
                    line = c.get("line") or thread.get("line")
            resolved = parse_marker(body, "codex-review:resolved")
            if resolved and not resolved.get("_invalid"):
                state = resolved.get("state") or state
                head_sha = resolved.get("head_sha") or head_sha
                reason = body
                if root_cause_key is None and resolved.get("root_cause_key"):
                    root_cause_key = resolved["root_cause_key"]
        out.append({
            "thread_id": thread.get("thread_id") or thread.get("id"),
            "root_cause_key": root_cause_key,
            "state": state,
            "reason": reason,
            "path": path,
            "line": line,
            "head_sha": head_sha,
        })
    return out


def build_resolved_memory(pr_context: dict[str, Any], threads: list[dict[str, Any]], config: dict[str, Any]) -> dict[str, Any]:
    policy = config.get("trusted", config)
    items = collect_resolved_memory(threads, policy)
    return {"schema_version": "resolve-gate-resolved-memory.v1", "head_sha": pr_context.get("head_sha"), "pr_number": pr_context.get("pr_number"), "items": items, "count": len(items)}


def write_resolved_memory(memory: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, memory, "resolve-gate-resolved-memory.v1")
