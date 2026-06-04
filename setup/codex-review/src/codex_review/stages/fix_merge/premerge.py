"""Premerge multiple patches."""
from __future__ import annotations
import subprocess, tempfile, shutil
from pathlib import Path
from typing import Any
from codex_review.core.artifacts import write_json, write_text
from codex_review.security.subprocess_env import sanitized_env


def apply_patches_in_temp_worktree(patches: list[str], repo_path: str | Path) -> dict[str, Any]:
    repo=Path(repo_path)
    tmp=Path(tempfile.mkdtemp(prefix="codex-premerge-"))
    try:
        if (repo/".git").exists():
            subprocess.run(["git","clone","--quiet",repo.as_posix(),tmp.as_posix()], check=True, env=sanitized_env())
        else:
            shutil.copytree(repo, tmp, dirs_exist_ok=True)
        results=[]
        for i,patch in enumerate(patches):
            proc=subprocess.run(["git","apply","--check","-"], input=patch, text=True, cwd=tmp, capture_output=True, env=sanitized_env())
            if proc.returncode==0:
                proc2=subprocess.run(["git","apply","-"], input=patch, text=True, cwd=tmp, capture_output=True, env=sanitized_env())
                results.append({"index":i,"ok":proc2.returncode==0,"stderr":proc2.stderr})
            else:
                results.append({"index":i,"ok":False,"stderr":proc.stderr})
        return {"temp_worktree": tmp.as_posix(), "results": results, "clean": all(r["ok"] for r in results)}
    except Exception as e:
        return {"temp_worktree": tmp.as_posix(), "results": [], "clean": False, "error": str(e)}


def build_premerge_report(results: dict[str, Any]) -> dict[str, Any]:
    return {"schema_version":"fix-merge-premerge-report.v1","clean":bool(results.get("clean")),"results":results.get("results", []),"temp_worktree":results.get("temp_worktree"),"error":results.get("error")}


def run_premerge_check(collection: dict[str, Any], repo_path: str | Path) -> dict[str, Any]:
    patches=[r.get("patch") or r.get("patch_text") or "" for r in collection.get("results", []) if r.get("status", "patched")=="patched"]
    return build_premerge_report(apply_patches_in_temp_worktree(patches, repo_path))


def write_premerge_report(report: dict[str, Any], out_path: str | Path) -> Path:
    return write_json(out_path, report, "fix-merge-premerge-report.v1")


def create_merged_patch_if_clean(temp_worktree: str | Path, out_patch: str | Path) -> Path:
    proc=subprocess.run(["git","diff","--binary"], cwd=Path(temp_worktree), capture_output=True, text=True, env=sanitized_env())
    return write_text(out_patch, proc.stdout)


def create_merged_fix_from_premerge(premerge_report: dict[str, Any], collection: dict[str, Any], pr_context: dict[str, Any] | None = None, out_path: str | Path | None = None) -> dict[str, Any]:
    """Create a fix_merge merged-fix artifact from clean deterministic premerge results.

    If no patches were available, returns a no_fix artifact. If premerge was not clean,
    returns a blocked artifact so the caller can route to a merge model or human review.
    """
    patches = [
        patch
        for r in collection.get("results", [])
        if r.get("status", "patched") == "patched"
        for patch in [r.get("patch") or r.get("patch_text") or ""]
        if patch
    ]
    if not patches:
        return {"schema_version": "fix-merge-merged-fix.v1", "status": "no_fix", "patch": "", "expected_head_sha": (pr_context or {}).get("head_sha"), "premerge_clean": bool(premerge_report.get("clean"))}
    if not premerge_report.get("clean"):
        return {"schema_version": "fix-merge-merged-fix.v1", "status": "blocked", "patch": "", "expected_head_sha": (pr_context or {}).get("head_sha"), "premerge_clean": False, "conflicts": premerge_report.get("results", [])}
    temp = premerge_report.get("temp_worktree")
    if temp:
        proc = subprocess.run(["git", "diff", "--binary"], cwd=Path(temp), capture_output=True, text=True, env=sanitized_env())
        patch = proc.stdout
    else:
        patch = _join_source_patches(patches)
    if not patch:
        patch = _join_source_patches(patches)
    merged = {"schema_version": "fix-merge-merged-fix.v1", "status": "ready", "patch": patch, "patch_text": patch, "expected_head_sha": (pr_context or {}).get("head_sha"), "premerge_clean": True, "source_patch_count": len(patches)}
    if out_path:
        write_json(out_path, merged, "fix-merge-merged-fix.v1")
    return merged


def _join_source_patches(patches: list[str]) -> str:
    if len(patches) == 1:
        return patches[0]
    return "\n".join(p.rstrip("\n") for p in patches if p).rstrip("\n") + "\n"
