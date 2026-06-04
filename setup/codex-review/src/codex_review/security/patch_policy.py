"""Autofix patch policy validation.

This module enforces mechanical invariants only: path allowlists, patch size,
forbidden git operations, secret-like material, and optional ``git apply`` checks.
Semantic risk words are emitted as advisory metadata for the AI review/fix loop;
they are deliberately not hard blockers here. OpenSpec-backed automation must be
able to implement spec-described work without substring or keyword vetoes in
trusted helper code.

Note: blast-radius caps (max_files / max_patch_bytes / max_commits / max_tasks)
were intentionally removed. They forced the self-healing autofix loop to escalate
to a human issue for work it could legitimately complete on large PRs. Runaway
loops are bounded instead by the convergence guards in ``loop.state`` (max_rounds,
oscillation/pingpong/revert detection) and by the semantic-safety model gate.
"""
from __future__ import annotations

import fnmatch
import re
import subprocess
from codex_review.security.subprocess_env import sanitized_env
from pathlib import Path
from typing import Any

from codex_review.core.errors import PolicyViolation
from codex_review.core.paths import safe_relative_path
from .redaction import scan_patch_for_secrets

DIFF_PATH_RE = re.compile(r"^diff --git a/(.+?) b/(.+)$")
FORBIDDEN_PATCH_MARKERS = ["Binary files ", "GIT binary patch", "rename from ", "rename to ", "copy from ", "copy to ", "new file mode 120000", "old mode ", "new mode "]


def _normalize(path: str) -> str:
    path = path.strip()
    if path == "/dev/null":
        return path
    if path.startswith("a/") or path.startswith("b/"):
        path = path[2:]
    return safe_relative_path(path)


def parse_patch_touched_files(patch_text: str) -> list[str]:
    touched=set()
    for line in (patch_text or "").splitlines():
        m=DIFF_PATH_RE.match(line)
        if m:
            for p in m.groups():
                if p != "/dev/null": touched.add(_normalize(p))
        elif line.startswith("+++ ") or line.startswith("--- "):
            p=line[4:].strip()
            if p != "/dev/null":
                touched.add(_normalize(p))
    return sorted(touched)


def _matches_any(path: str, patterns: list[str]) -> bool:
    for pattern in patterns or []:
        if not pattern:
            continue
        prefix = pattern if str(pattern).endswith("/") else str(pattern).rstrip("/") + "/"
        if path == str(pattern).rstrip("/") or path.startswith(prefix) or fnmatch.fnmatch(path, str(pattern)):
            return True
    return False


def assert_allowed_paths(touched_files: list[str], policy: dict[str, Any]) -> None:
    allowed = policy.get("allowed_files") or []
    allowed_prefixes = policy.get("allowed_prefixes") or []
    forbidden = set(policy.get("forbidden_files") or [])
    forbidden_prefixes = policy.get("forbidden_prefixes") or []
    for path in touched_files:
        if path in forbidden or _matches_any(path, forbidden_prefixes):
            raise PolicyViolation(f"patch touches forbidden path: {path}")
        if allowed or allowed_prefixes:
            if path not in allowed and not _matches_any(path, allowed_prefixes):
                raise PolicyViolation(f"patch touches path outside allowlist: {path}")


def assert_no_binary_mode_rename_or_symlink(patch_text: str) -> None:
    for marker in FORBIDDEN_PATCH_MARKERS:
        if marker in (patch_text or ""):
            raise PolicyViolation(f"forbidden patch operation detected: {marker.strip()}")


def _added_lines(patch_text: str) -> list[tuple[int, str]]:
    lines: list[tuple[int, str]] = []
    for line_no, line in enumerate((patch_text or "").splitlines(), 1):
        if line.startswith("+") and not line.startswith("+++"):
            lines.append((line_no, line[1:]))
    return lines


def collect_dangerous_keyword_warnings(patch_text: str, policy: dict[str, Any]) -> list[dict[str, Any]]:
    """Return advisory semantic-risk keyword hits without blocking the patch.

    These keywords are prompt hints for model stages. Trusted Python helpers must
    not reject OpenSpec implementation patches because a word like ``auth`` is a
    substring of harmless documentation such as ``authoritative``.
    """
    keywords=[str(k).lower() for k in policy.get("dangerous_keywords", []) or []]
    warnings: list[dict[str, Any]] = []
    for line_no, line in _added_lines(patch_text):
        lower=line.lower()
        for keyword in keywords:
            if keyword and keyword in lower:
                warnings.append({"line": line_no, "keyword": keyword, "kind": "semantic_keyword"})
    return warnings


def collect_public_api_risk_warnings(patch_text: str, source_context: dict[str, Any] | None, policy: dict[str, Any]) -> list[dict[str, Any]]:
    if policy.get("allow_public_api_changes"):
        return []
    risky=["pub fn ", "public ", "export ", "module.exports", "@api", "serde", "signature", "nonce", "signing"]
    warnings: list[dict[str, Any]] = []
    for line_no, line in _added_lines(patch_text):
        lower=line.lower()
        for token in risky:
            if token in lower:
                warnings.append({"line": line_no, "token": token.strip(), "kind": "public_api_or_protocol_semantic_risk"})
    return warnings


# Backward-compatible names kept for callers/tests that import them. They now
# return advisory findings instead of raising policy violations.
def assert_no_dangerous_keyword_changes(patch_text: str, policy: dict[str, Any]) -> list[dict[str, Any]]:
    return collect_dangerous_keyword_warnings(patch_text, policy)


def assert_no_public_api_risk(patch_text: str, source_context: dict[str, Any] | None, policy: dict[str, Any]) -> list[dict[str, Any]]:
    return collect_public_api_risk_warnings(patch_text, source_context, policy)


def git_apply_check(patch_text: str, repo_path: str | Path) -> None:
    if not repo_path:
        return
    p=Path(repo_path)
    if not (p / ".git").exists():
        return
    proc=subprocess.run(["git", "apply", "--check", "-"], input=patch_text, text=True, cwd=p, capture_output=True, env=sanitized_env())
    if proc.returncode != 0:
        raise PolicyViolation(f"git apply --check failed: {proc.stderr.strip()}")


def validate_patch_policy(patch_text: str, policy: dict[str, Any], context: dict[str, Any] | None = None) -> dict[str, Any]:
    context=context or {}
    assert_no_binary_mode_rename_or_symlink(patch_text)
    touched=parse_patch_touched_files(patch_text)
    assert_allowed_paths(touched, policy)
    findings=scan_patch_for_secrets(patch_text)
    if findings:
        raise PolicyViolation(f"secret-like material detected in patch: {findings[:3]}")
    semantic_warnings = [
        *collect_dangerous_keyword_warnings(patch_text, policy),
        *collect_public_api_risk_warnings(patch_text, context.get("source_context"), policy),
    ]
    if context.get("repo_path"):
        git_apply_check(patch_text, context["repo_path"])
    return {
        "ok": True,
        "touched_files": touched,
        "patch_bytes": len((patch_text or '').encode('utf-8')),
        "semantic_risk_warnings": semantic_warnings,
        "semantic_risk_warning_count": len(semantic_warnings),
    }
