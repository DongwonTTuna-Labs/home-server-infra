"""AI semantic safety review for exact merged patches.

This stage is intentionally semantic, not string-keyword based. Scripted policy may
flag suspicious surfaces as advisory evidence, but Stage07 write permission is only
allowed when a model has approved the exact merged patch hash produced by Stage06.
"""
from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

from codex_review.context.budget import compact_json
from codex_review.context.budget import estimate_tokens
from codex_review.core.errors import ValidationError
from codex_review.core.output import write_output
from codex_review.patches.commit_plan import normalize_commit_plan


def patch_text_from_merged_fix(merged_fix: dict[str, Any]) -> str:
    if merged_fix.get("patch_path"):
        return Path(str(merged_fix["patch_path"])).read_text(encoding="utf-8")
    return str(merged_fix.get("patch") or merged_fix.get("patch_text") or "")


def sha256_text(text: str) -> str:
    return hashlib.sha256((text or "").encode("utf-8")).hexdigest()


def _compact_json_like(value: Any, *, limit: int = 20000) -> str:
    return compact_json(value, max_chars=limit)


def build_semantic_patch_safety_prompt(
    merged_fix: dict[str, Any],
    pr_context: dict[str, Any],
    docs_context: str,
    *,
    repo_path: str | Path | None = None,
    token_budget: int | None = None,
) -> str:
    patch = patch_text_from_merged_fix(merged_fix)
    patch_hash = sha256_text(patch)
    status = str(merged_fix.get("status") or "")
    title = pr_context.get("title") or pr_context.get("pr_title") or ""
    body = pr_context.get("body") or pr_context.get("pr_body") or ""
    changed_files = pr_context.get("files") or pr_context.get("changed_files") or []

    repo_note = ""
    if repo_path and Path(repo_path).exists():
        repo_note = f"Repository checkout available read-only at: {Path(repo_path).resolve()}"

    prompt = f"""# Stage06 Semantic Patch Safety Review

You are the semantic safety reviewer for an autonomous OpenSpec-driven PR completion loop.
This is not a keyword blocker. Review the exact merged patch semantically against the PR intent,
linked OpenSpec artifacts, design/chief decisions embedded in the artifacts, and operational safety.

## Required decision
Return JSON matching `fix-merge-semantic-patch-safety.v1`.

Approve only when all of the following are true:
1. The exact patch hash below matches the patch you reviewed.
2. The patch is within the PR/OpenSpec scope and advances the stated implementation.
3. The patch does not introduce credential exfiltration, unsafe network side effects, workflow permission escalation,
   malicious dependency/script execution, or hidden bypasses.
4. Any advisory risky surfaces such as auth, signing, nonce, token, permissions, public API, workflow, or shell code
   are semantically justified by the OpenSpec/design and have adequate tests or verification.
5. No concrete blocker requires human credentials, out-of-repo mutation, or external secrets.

Also produce a semantic `commit_plan` for approved patches:
- Group edits by coherent task/root cause, not necessarily by file and not as one generic blob.
- Each entry must have a Conventional Commit `subject`, an explanatory `body`, and the exact repository `paths`.
- Never use a generic subject such as `Codex Review Autofix`; the subject must describe the actual change.
- The union of all commit_plan paths must exactly cover the patch paths you approve.

Reject rather than guessing when the patch cannot be semantically reviewed. Do not reject merely because risky words appear;
reject only with evidence from the patch and artifacts. If there is no patch because Stage06 status is no_fix/blocked,
use status `not_required`, approved false, the exact patch_hash, and an empty commit_plan.

## Exact merged patch identity
- patch_hash_sha256: `{patch_hash}`
- fix_merge_status: `{status}`
- expected_head_sha: `{merged_fix.get('expected_head_sha') or pr_context.get('head_sha') or ''}`

## PR title
{title}

## PR body
{body}

## Changed files/context
{_compact_json_like(changed_files, limit=12000)}

## OpenSpec and repository docs context
{docs_context}

## Stage06 merged fix artifact summary
{_compact_json_like({k: v for k, v in merged_fix.items() if k not in {'patch', 'patch_text'}}, limit=20000)}

## Exact merged patch
```diff
{patch}
```

{repo_note}
"""
    # Fail-closed: a single-pass semantic review can only be trusted if the whole
    # patch + context fits the model window. If it does not, refuse to emit a prompt
    # rather than silently overflow and risk approving a patch the model never fully read.
    if token_budget and estimate_tokens(prompt) > int(token_budget):
        raise ValidationError(
            "patch too large for single-pass semantic safety review: "
            f"estimated {estimate_tokens(prompt)} tokens exceeds budget {int(token_budget)}. "
            "Reduce the merged patch scope (split the fix) so the exact patch can be reviewed in one pass."
        )
    return prompt


def validate_semantic_patch_safety_result(raw: dict[str, Any], merged_fix: dict[str, Any]) -> dict[str, Any]:
    patch = patch_text_from_merged_fix(merged_fix)
    expected_hash = sha256_text(patch)
    status = str(merged_fix.get("status") or "")
    out = dict(raw or {})
    out["schema_version"] = "fix-merge-semantic-patch-safety.v1"
    out.setdefault("patch_hash", expected_hash)
    out.setdefault("summary", "")
    out.setdefault("blocking_reason", None)
    out.setdefault("reviewed_criteria", [])
    out.setdefault("semantic_findings", [])
    out.setdefault("commit_plan", [])

    if not patch or status in {"no_fix", "blocked"}:
        out["status"] = "not_required"
        out["approved"] = False
        out["patch_hash"] = expected_hash
        out.setdefault("summary", "No ready patch requires semantic push approval.")
        return out

    if out.get("patch_hash") != expected_hash:
        raise ValidationError(f"semantic safety patch_hash mismatch: expected {expected_hash}, got {out.get('patch_hash')}")

    decision = out.get("status")
    if decision not in {"approved", "rejected", "needs_issue"}:
        raise ValidationError("semantic safety status must be approved, rejected, or needs_issue for ready patches")
    approved = bool(out.get("approved"))
    if decision == "approved" and not approved:
        raise ValidationError("semantic safety status approved requires approved=true")
    if approved and decision != "approved":
        raise ValidationError("semantic safety approved=true requires status=approved")
    if decision != "approved" and not str(out.get("blocking_reason") or out.get("summary") or "").strip():
        raise ValidationError("semantic safety rejection requires blocking_reason or summary")
    if decision == "approved":
        out["commit_plan"] = normalize_commit_plan(out.get("commit_plan"), patch)
    return out


def write_semantic_safety_outputs(result: dict[str, Any]) -> dict[str, Any]:
    status = str(result.get("status") or "unknown")
    approved = bool(result.get("approved"))
    patch_hash = str(result.get("patch_hash") or "")
    write_output("semantic_safety_status", status)
    write_output("semantic_safety_approved", str(approved).lower())
    if patch_hash:
        write_output("semantic_safety_patch_hash", patch_hash)
    return {"semantic_safety_status": status, "semantic_safety_approved": approved, "semantic_safety_patch_hash": patch_hash}
