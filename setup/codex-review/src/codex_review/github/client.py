"""Small GitHub REST/GraphQL client using the Python standard library."""
from __future__ import annotations

import json
import os
import urllib.parse
import urllib.request
from typing import Any

from codex_review.core.errors import GitHubError

API_ROOT = os.environ.get("GITHUB_API_URL", "https://api.github.com")


def api_root() -> str:
    return os.environ.get("GITHUB_API_URL", API_ROOT).rstrip("/")


def build_headers(token: str | None) -> dict[str, str]:
    headers = {
        "Accept": "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
        "User-Agent": "codex-review-v3",
    }
    if token:
        headers["Authorization"] = f"Bearer {token}"
    return headers


def _request(method: str, url: str, token: str | None, body: Any = None) -> tuple[Any, dict[str, str], int]:
    data = None
    headers = build_headers(token)
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method.upper())
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:  # nosec - URL is GitHub API or test URL
            raw = resp.read().decode("utf-8")
            parsed = json.loads(raw) if raw else None
            return parsed, dict(resp.headers), resp.status
    except urllib.error.HTTPError as exc:
        text = exc.read().decode("utf-8", errors="replace")
        raise GitHubError(f"GitHub API {method} {url} failed: {exc.code} {text[:500]}") from exc
    except urllib.error.URLError as exc:
        raise GitHubError(f"GitHub API {method} {url} failed: {exc}") from exc


def rest_request(method: str, url: str, token: str | None, body: Any = None) -> Any:
    parsed, _headers, _status = _request(method, url, token, body)
    return parsed


def _next_link(link_header: str | None) -> str | None:
    if not link_header:
        return None
    for part in link_header.split(","):
        if 'rel="next"' in part:
            start=part.find("<"); end=part.find(">")
            if start != -1 and end != -1:
                return part[start+1:end]
    return None


def rest_paginated(url: str, token: str | None, params: dict[str, Any] | None = None) -> list[Any]:
    if params:
        sep = "&" if "?" in url else "?"
        url = url + sep + urllib.parse.urlencode(params)
    items: list[Any] = []
    while url:
        payload, headers, _ = _request("GET", url, token)
        if isinstance(payload, list):
            items.extend(payload)
        elif isinstance(payload, dict) and "items" in payload:
            items.extend(payload["items"])
        elif payload is not None:
            items.append(payload)
        url = _next_link(headers.get("Link"))
    return items


def graphql_request(query: str, variables: dict[str, Any] | None, token: str | None) -> dict[str, Any]:
    payload = rest_request("POST", f"{api_root()}/graphql", token, {"query": query, "variables": variables or {}})
    if payload and payload.get("errors"):
        raise GitHubError(f"GraphQL errors: {payload['errors']}")
    return payload.get("data", {}) if isinstance(payload, dict) else {}


def github_api_url(owner: str, repo: str, path: str) -> str:
    base = f"{api_root()}/repos/{urllib.parse.quote(owner, safe='')}/{urllib.parse.quote(repo, safe='')}"
    if not path:
        return base
    clean = path if path.startswith("/") else f"/{path}"
    return f"{base}{clean}"
