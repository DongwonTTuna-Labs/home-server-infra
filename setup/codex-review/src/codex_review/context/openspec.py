"""OpenSpec context discovery for PR-scoped review automation."""
from __future__ import annotations

import base64
import hashlib
import re
import urllib.parse
from pathlib import Path
from typing import Any

from codex_review.core.errors import GitHubError
from codex_review.github.client import github_api_url, rest_request
from codex_review.github.pull_requests import get_pull_request, list_pull_request_files
from codex_review.memory.paths import is_memory_path

CHANGE_DOC_NAMES = ("proposal.md", "design.md", "tasks.md")
CONFIG_PATHS = (".openspec.yaml", "openspec/config.yaml")
OPEN_SPEC_PREFIXES = ("openspec/", ".openspec/")
SCHEMA_VERSION = "openspec-context.v1"


def _repo_parts(pr_context: dict[str, Any], owner: str | None = None, repo: str | None = None) -> tuple[str | None, str | None]:
    out_owner = owner or pr_context.get("owner")
    out_repo = repo or pr_context.get("repo")
    repository = pr_context.get("repository") or pr_context.get("base_repo_full_name")
    if (not out_owner or not out_repo) and isinstance(repository, str) and "/" in repository:
        out_owner, out_repo = repository.split("/", 1)
    return out_owner, out_repo


def _normalize_path(path: str) -> str:
    clean = urllib.parse.unquote(str(path or "")).strip().strip("`'\"()[]{}<>.,")
    clean = clean.lstrip("/")
    while clean.startswith("./"):
        clean = clean[2:]
    return clean


def _is_openspec_path(path: str) -> bool:
    clean = _normalize_path(path)
    return clean.startswith(OPEN_SPEC_PREFIXES) or clean in CONFIG_PATHS


def _dedupe_sources(sources: list[dict[str, Any]]) -> list[dict[str, Any]]:
    seen: set[tuple[Any, ...]] = set()
    out: list[dict[str, Any]] = []
    for source in sources:
        key = (
            source.get("type"),
            source.get("owner"),
            source.get("repo"),
            source.get("pr_number"),
            source.get("ref"),
            source.get("path"),
            source.get("url"),
        )
        if key in seen:
            continue
        seen.add(key)
        out.append(source)
    return out


def _extract_path_sources(text: str) -> list[dict[str, Any]]:
    sources: list[dict[str, Any]] = []
    for match in re.finditer(r"(?<![\w./-])((?:\.openspec|openspec)/[A-Za-z0-9._/@+\-=]+)", text or ""):
        path = _normalize_path(match.group(1))
        if _is_openspec_path(path):
            sources.append({"type": "path", "path": path})
    return sources


def _openspec_path_from_parts(parts: list[str]) -> tuple[str | None, str | None]:
    for index, part in enumerate(parts):
        if part in {"openspec", ".openspec"}:
            return "/".join(parts[index:]), "/".join(parts[:index])
    return None, None


def _extract_github_source(url: str, owner: str | None, repo: str | None) -> dict[str, Any] | None:
    parsed = urllib.parse.urlparse(url)
    host = parsed.netloc.lower()
    parts = [urllib.parse.unquote(part) for part in parsed.path.strip("/").split("/") if part]
    if host == "github.com" and len(parts) >= 4:
        url_owner, url_repo = parts[0], parts[1]
        if parts[2] == "pull" and len(parts) >= 4 and parts[3].isdigit():
            return {"type": "github_pr", "owner": url_owner, "repo": url_repo, "pr_number": int(parts[3]), "url": url}
        if parts[2] in {"blob", "raw", "tree"}:
            path, ref_prefix = _openspec_path_from_parts(parts[3:])
            if path and _is_openspec_path(path):
                return {"type": "github_file", "owner": url_owner, "repo": url_repo, "ref": ref_prefix, "path": path, "url": url}
    if host == "raw.githubusercontent.com" and len(parts) >= 4:
        url_owner, url_repo = parts[0], parts[1]
        path, ref_prefix = _openspec_path_from_parts(parts[2:])
        if path and _is_openspec_path(path):
            return {"type": "github_file", "owner": url_owner, "repo": url_repo, "ref": ref_prefix, "path": path, "url": url}
    return None


def extract_openspec_sources(text: str, *, owner: str | None = None, repo: str | None = None) -> list[dict[str, Any]]:
    """Extract OpenSpec path, file URL and PR URL hints from PR title/body text."""
    sources = _extract_path_sources(text)
    for match in re.finditer(r"https?://[^\s`'\"<>)]*", text or ""):
        source = _extract_github_source(match.group(0).rstrip(".,;"), owner, repo)
        if source:
            sources.append(source)
    return _dedupe_sources(sources)


def _relative(path: Path, root: Path) -> str:
    try:
        return path.relative_to(root).as_posix()
    except ValueError:
        return path.as_posix()


def _candidate_local_files(path: Path, repo_root: Path) -> list[Path]:
    if path.is_file():
        return [path]
    if not path.is_dir():
        return []
    candidates: list[Path] = []
    for name in CHANGE_DOC_NAMES:
        candidate = path / name
        if candidate.is_file():
            candidates.append(candidate)
    specs_dir = path / "specs"
    if specs_dir.is_dir():
        candidates.extend(sorted(specs_dir.rglob("*.md")))
    for config_path in CONFIG_PATHS:
        candidate = repo_root / config_path
        if candidate.is_file():
            candidates.append(candidate)
    return candidates


def _read_local_documents(source: dict[str, Any], repo_root: Path, budget: int) -> list[dict[str, Any]]:
    path = repo_root / _normalize_path(str(source.get("path") or ""))
    docs: list[dict[str, Any]] = []
    for file_path in _candidate_local_files(path, repo_root):
        text = file_path.read_text(encoding="utf-8", errors="replace")
        docs.append(
            {
                "source_type": source.get("type"),
                "path": _relative(file_path, repo_root),
                "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
                "content": text[:budget],
                "truncated": len(text) > budget,
            }
        )
    return docs


def _decode_contents_response(payload: Any) -> str | None:
    if not isinstance(payload, dict):
        return None
    content = payload.get("content")
    if not isinstance(content, str):
        return None
    try:
        return base64.b64decode(content.encode("ascii"), validate=False).decode("utf-8", errors="replace")
    except Exception:
        return None


def _read_github_file(source: dict[str, Any], token: str | None, budget: int) -> list[dict[str, Any]]:
    owner = source.get("owner")
    repo = source.get("repo")
    path = _normalize_path(str(source.get("path") or ""))
    if not owner or not repo or not path or not token:
        return []
    url = github_api_url(str(owner), str(repo), f"/contents/{urllib.parse.quote(path, safe='/')}")
    ref = source.get("ref")
    if ref:
        url = f"{url}?{urllib.parse.urlencode({'ref': str(ref)})}"
    payload = rest_request("GET", url, token)
    text = _decode_contents_response(payload)
    if text is None:
        return []
    return [
        {
            "source_type": source.get("type"),
            "owner": owner,
            "repo": repo,
            "path": path,
            "ref": ref,
            "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
            "content": text[:budget],
            "truncated": len(text) > budget,
        }
    ]


def _read_github_documents(source: dict[str, Any], token: str | None, budget: int) -> list[dict[str, Any]]:
    path = _normalize_path(str(source.get("path") or ""))
    if not path or Path(path).suffix in {".md", ".yaml", ".yml"}:
        return _read_github_file(source, token, budget)
    owner = source.get("owner")
    repo = source.get("repo")
    ref = str(source.get("ref") or "")
    if not owner or not repo or not ref or not token:
        return []
    url = github_api_url(str(owner), str(repo), f"/git/trees/{urllib.parse.quote(ref, safe='')}?recursive=1")
    tree = rest_request("GET", url, token)
    items = tree.get("tree", []) if isinstance(tree, dict) else []
    wanted: list[str] = []
    direct = {f"{path.rstrip('/')}/{name}" for name in CHANGE_DOC_NAMES}
    for item in items:
        item_path = _normalize_path(str(item.get("path") or ""))
        if item_path in direct or (item_path.startswith(f"{path.rstrip('/')}/specs/") and item_path.endswith(".md")) or item_path in CONFIG_PATHS:
            wanted.append(item_path)
    docs: list[dict[str, Any]] = []
    for item_path in sorted(set(wanted)):
        docs.extend(_read_github_file({**source, "path": item_path}, token, budget))
    return docs


def _sources_from_changed_files(pr_context: dict[str, Any]) -> list[dict[str, Any]]:
    sources: list[dict[str, Any]] = []
    for item in pr_context.get("changed_files_summary", []) or []:
        path = _normalize_path(str(item.get("filename") or ""))
        if is_memory_path(path):
            continue
        if _is_openspec_path(path):
            sources.append({"type": "path", "path": path})
    return sources


def _sources_from_github_pr(source: dict[str, Any], token: str | None) -> list[dict[str, Any]]:
    owner = source.get("owner")
    repo = source.get("repo")
    pr_number = source.get("pr_number")
    if not owner or not repo or not pr_number or not token:
        return []
    pr = get_pull_request(str(owner), str(repo), int(pr_number), token)
    nested = extract_openspec_sources(
        "\n".join([str(pr.get("title") or ""), str(pr.get("body") or "")]),
        owner=str(owner),
        repo=str(repo),
    )
    try:
        files = list_pull_request_files(str(owner), str(repo), int(pr_number), token)
    except GitHubError:
        files = []
    head_sha = ((pr.get("head") or {}).get("sha") if isinstance(pr, dict) else None) or source.get("ref")
    for file_info in files:
        path = _normalize_path(str(file_info.get("filename") or ""))
        if is_memory_path(path):
            continue
        if _is_openspec_path(path):
            nested.append({"type": "github_file", "owner": owner, "repo": repo, "ref": head_sha, "path": path})
    return nested


def _summary_value(item: dict[str, Any]) -> str:
    path = item.get("path")
    owner = item.get("owner")
    repo = item.get("repo")
    if path and owner and repo:
        return f"{owner}/{repo}:{path}"
    if path:
        return str(path)
    if item.get("url"):
        return str(item["url"])
    if item.get("pr_number") and owner and repo:
        return f"{owner}/{repo}#PR {item.get('pr_number')}"
    if item.get("pr_number"):
        return f"PR #{item.get('pr_number')}"
    return "OpenSpec source"


def _source_summary(documents: list[dict[str, Any]], sources: list[dict[str, Any]]) -> list[str]:
    values = [_summary_value(doc) for doc in documents if doc.get("path")]
    if not values:
        values = [_summary_value(src) for src in sources]
    seen: set[str] = set()
    out: list[str] = []
    for value in values:
        if value and value not in seen:
            seen.add(value)
            out.append(value)
    return out


def collect_openspec_context(
    pr_context: dict[str, Any],
    repo_path: str | Path = ".",
    token: str | None = None,
    *,
    budget: int = 40000,
) -> dict[str, Any]:
    """Collect linked OpenSpec artifacts from PR text, same-repo paths and PR links."""
    owner, repo = _repo_parts(pr_context)
    source_text = "\n".join([str(pr_context.get("title") or ""), str(pr_context.get("body") or "")])
    sources = extract_openspec_sources(source_text, owner=owner, repo=repo)
    sources.extend(_sources_from_changed_files(pr_context))
    expanded: list[dict[str, Any]] = []
    for source in sources:
        if source.get("type") == "github_pr":
            expanded.extend(_sources_from_github_pr(source, token))
    sources = _dedupe_sources([*sources, *expanded])

    repo_root = Path(repo_path)
    documents: list[dict[str, Any]] = []
    for source in sources:
        if source.get("type") == "path":
            local_docs = _read_local_documents(source, repo_root, budget)
            if local_docs:
                documents.extend(local_docs)
            elif token and owner and repo and pr_context.get("head_sha"):
                documents.extend(
                    _read_github_documents(
                        {"type": "github_file", "owner": owner, "repo": repo, "ref": pr_context.get("head_sha"), "path": source.get("path")},
                        token,
                        budget,
                    )
                )
        elif source.get("type") == "github_file":
            remote_docs = _read_github_documents(source, token, budget)
            documents.extend(remote_docs or _read_local_documents({"type": "path", "path": source.get("path")}, repo_root, budget))

    seen_docs: set[tuple[str, str]] = set()
    deduped_docs: list[dict[str, Any]] = []
    for doc in documents:
        key = (str(doc.get("path")), str(doc.get("sha256")))
        if key in seen_docs:
            continue
        seen_docs.add(key)
        deduped_docs.append(doc)

    present = bool(deduped_docs)
    if present:
        status = "ready"
        decision = "use_openspec_context"
        missing_reason = None
    elif sources:
        status = "unresolved_openspec_source"
        decision = "missing_openspec_spec"
        missing_reason = "OpenSpec source was referenced but no readable artifacts were found"
    else:
        status = "missing_openspec_spec"
        decision = "missing_openspec_spec"
        missing_reason = "PR title/body and changed files do not reference OpenSpec artifacts"

    return {
        "schema_version": SCHEMA_VERSION,
        "present": present,
        "status": status,
        "decision": decision,
        "owner": owner,
        "repo": repo,
        "sources": sources,
        "documents": deduped_docs,
        "document_count": len(deduped_docs),
        "source_summary": _source_summary(deduped_docs, sources),
        "missing_reason": missing_reason,
    }


_SECTION_BY_BASENAME = {
    "tasks.md": "tasks",
    "proposal.md": "proposal",
    "design.md": "design",
    "config.yaml": "config",
    "config.yml": "config",
}


def _doc_section(path: str) -> str:
    name = Path(str(path or "")).name
    if name in _SECTION_BY_BASENAME:
        return _SECTION_BY_BASENAME[name]
    if "/specs/" in str(path or "") or name == "spec.md":
        return "spec"
    return "other"


def sections_for_stage(stage: str | None) -> set[str] | None:
    """Which OpenSpec doc sections a stage needs (None => all).

    Review/techlead reason about intent (proposal + spec); design/fix stages need the
    full implementation contract (tasks + spec + design). Scoping avoids inlining the
    entire OpenSpec change (which can be 100k+ tokens) into every stage's prompt.
    """
    if not stage:
        return None
    s = str(stage)
    if s.startswith(("review", "techlead")) or s in {"review", "techlead"}:
        return {"proposal", "spec", "other"}
    if s.startswith(("design", "design_chief", "fix_dispatch", "fix_merge")) or s in {"design", "fix"}:
        return {"tasks", "spec", "design", "proposal", "config", "other"}
    return None


def render_openspec_context_markdown(
    context: dict[str, Any],
    *,
    sections: set[str] | list[str] | None = None,
    budget_tokens: int | None = None,
) -> str:
    lines = ["## OpenSpec context", ""]
    if not context.get("present"):
        lines.extend(
            [
                f"Status: `{context.get('status') or 'missing_openspec_spec'}`",
                f"Decision: `{context.get('decision') or 'missing_openspec_spec'}`",
            ]
        )
        if context.get("missing_reason"):
            lines.append(f"Missing reason: {context['missing_reason']}")
        return "\n".join(lines).rstrip() + "\n"

    lines.append("Status: `ready`")
    lines.append("")
    lines.append("Sources:")
    for source in context.get("source_summary", []):
        lines.append(f"- {source}")

    documents = context.get("documents", []) or []
    if sections is not None:
        wanted = set(sections)
        selected = [doc for doc in documents if _doc_section(doc.get("path")) in wanted]
        documents = selected or documents  # never render an empty context when docs exist
    for doc in documents:
        path = doc.get("path") or "OpenSpec document"
        lines.extend(["", f"### {path}", "", str(doc.get("content") or "").rstrip()])
        if doc.get("truncated"):
            lines.append("\n[truncated]")
    rendered = "\n".join(lines).rstrip() + "\n"
    if budget_tokens:
        from codex_review.context.budget import fit_to_budget

        fitted, truncated = fit_to_budget(rendered, int(budget_tokens))
        if truncated:
            rendered = fitted.rstrip() + "\n\n[openspec context truncated to fit token budget]\n"
    return rendered
