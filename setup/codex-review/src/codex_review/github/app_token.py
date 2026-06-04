"""GitHub App token helpers."""
from __future__ import annotations

import base64
import json
import os
import time
import urllib.parse
from typing import Any

from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import padding

from codex_review.core.errors import ValidationError
from .client import api_root, github_api_url, rest_request


def _b64(data: bytes) -> str:
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode("ascii")


def _api_root() -> str:
    return api_root()


def load_app_credentials_from_env() -> dict[str, str]:
    app_id = (
        os.environ.get("CODEX_REVIEW_GITHUB_APP_ID")
        or os.environ.get("CODEX_REVIEW_APP_ID")
        or os.environ.get("GITHUB_APP_ID")
        or os.environ.get("CODEX_APP_ID")
        or os.environ.get("APP_ID")
    )
    key = (
        os.environ.get("CODEX_REVIEW_GITHUB_APP_PRIVATE_KEY")
        or os.environ.get("CODEX_REVIEW_APP_PRIVATE_KEY")
        or os.environ.get("GITHUB_APP_PRIVATE_KEY")
        or os.environ.get("CODEX_APP_PRIVATE_KEY")
        or os.environ.get("APP_PRIVATE_KEY")
    )
    if not app_id or not key:
        raise ValidationError(
            "GitHub App credentials are required: set CODEX_REVIEW_GITHUB_APP_ID and "
            "CODEX_REVIEW_GITHUB_APP_PRIVATE_KEY (or compatible GITHUB_APP_* env vars)"
        )
    return {"app_id": str(app_id), "private_key": str(key).replace("\\n", "\n")}


def create_jwt(app_id: str, private_key: str) -> str:
    header = {"alg": "RS256", "typ": "JWT"}
    now = int(time.time())
    payload = {"iat": now - 60, "exp": now + 540, "iss": str(app_id)}
    signing_input = f"{_b64(json.dumps(header, separators=(',', ':')).encode())}.{_b64(json.dumps(payload, separators=(',', ':')).encode())}".encode()
    key = serialization.load_pem_private_key(private_key.encode(), password=None)
    sig = key.sign(signing_input, padding.PKCS1v15(), hashes.SHA256())
    return signing_input.decode() + "." + _b64(sig)


def get_installation_id(owner: str, repo: str, jwt: str) -> int:
    if not owner or not repo:
        raise ValidationError("owner and repo are required to resolve GitHub App installation")
    owner_q = urllib.parse.quote(str(owner), safe="")
    repo_q = urllib.parse.quote(str(repo), safe="")
    data = rest_request("GET", f"{_api_root()}/repos/{owner_q}/{repo_q}/installation", jwt)
    return int(data["id"])


def _permission_satisfies(actual: str | None, expected: str) -> bool:
    order = {"none": 0, "read": 1, "write": 2}
    return order.get(str(actual or "none"), 0) >= order.get(str(expected or "none"), 0)


def _validate_response_permissions(response_permissions: dict[str, Any], required: dict[str, str]) -> None:
    for key, expected in (required or {}).items():
        actual = response_permissions.get(key)
        if not _permission_satisfies(actual, expected):
            raise ValidationError(f"GitHub App token response lacks {key}:{expected}; got {actual or 'none'}")




def _declared_permission_maps_from_env() -> list[dict[str, Any]]:
    """Read permissions asserted by the token-minting step, if present.

    GitHub's installation-token introspection endpoints prove token type and repo
    membership, but they do not always expose every fine-grained permission. The
    workflow therefore threads the permissions returned by the token creation
    response into CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON. This is not used on its
    own: repository membership is still actively checked against GitHub for the
    presented token.
    """
    maps: list[dict[str, Any]] = []
    for name in ("CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON", "CODEX_REVIEW_APP_TOKEN_PERMISSIONS"):
        raw = os.environ.get(name)
        if not raw:
            continue
        try:
            parsed = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise ValidationError(f"{name} must be valid JSON") from exc
        if isinstance(parsed, dict):
            maps.append(parsed)
    return maps


def _coarse_permission_satisfies(perm_map: dict[str, Any], key: str, expected: str) -> bool:
    """Check granular App permission maps and repository permission maps."""
    actual = perm_map.get(key)
    if isinstance(actual, str) and _permission_satisfies(actual, expected):
        return True
    if isinstance(actual, bool):
        if expected == "read":
            return bool(actual)
        if expected == "write":
            return bool(actual)
    # Repository objects expose coarse pull/push/admin booleans rather than the
    # full GitHub App permission map. Use those only where they are meaningful.
    admin = bool(perm_map.get("admin"))
    push = bool(perm_map.get("push"))
    pull = bool(perm_map.get("pull"))
    if key == "contents":
        if expected == "write":
            return admin or push
        if expected == "read":
            return admin or push or pull
    if key in {"issues", "pull_requests"} and expected == "read":
        return admin or push or pull
    return False


def _validate_required_permission_maps(permission_maps: list[dict[str, Any]], required: dict[str, str], *, strict: bool = True) -> dict[str, str]:
    satisfied: dict[str, str] = {}
    for key, expected in (required or {}).items():
        if expected == "none":
            continue
        source = None
        for idx, perm_map in enumerate(permission_maps):
            if _coarse_permission_satisfies(perm_map, key, expected):
                source = f"permission_map[{idx}]"
                break
        if source is None:
            if strict:
                raise ValidationError(f"GitHub App installation token permission preflight failed for {key}:{expected}")
        else:
            satisfied[key] = source
    return satisfied

def create_installation_token(
    installation_id: int,
    permissions: dict[str, str],
    jwt: str,
    *,
    repositories: list[str] | None = None,
    repository_ids: list[int] | None = None,
) -> str:
    if not installation_id:
        raise ValidationError("installation_id is required")
    if not isinstance(permissions, dict):
        raise ValidationError("permissions must be a dict")
    body: dict[str, Any] = {"permissions": permissions}
    if repository_ids:
        body["repository_ids"] = [int(x) for x in repository_ids]
    elif repositories:
        body["repositories"] = [str(x) for x in repositories]
    data = rest_request("POST", f"{_api_root()}/app/installations/{int(installation_id)}/access_tokens", jwt, body)
    token = data.get("token") if isinstance(data, dict) else None
    if not token:
        raise ValidationError("GitHub App installation token response did not include token")
    _validate_response_permissions(data.get("permissions") or {}, permissions)
    return str(token)


def create_installation_token_for_repo(owner: str, repo: str, permissions: dict[str, str]) -> str:
    """Create a GitHub App installation token scoped to one repository."""
    creds = load_app_credentials_from_env()
    jwt = create_jwt(creds["app_id"], creds["private_key"])
    installation_id = get_installation_id(owner, repo, jwt)
    # The installation access token API accepts repository names for scoping when
    # the installation can access multiple repositories. This prevents a token
    # minted for one PR from being valid across every repository in the app install.
    return create_installation_token(installation_id, permissions, jwt, repositories=[repo])


def permissions_for_write_mode(mode: str | None) -> dict[str, str]:
    if mode in {"label-ops", "labels", "finalize"}:
        return {"contents": "read", "pull_requests": "write", "issues": "read"}
    if mode == "push":
        return {"contents": "write", "pull_requests": "read", "issues": "read"}
    if mode == "dispatch":
        return {"contents": "write", "pull_requests": "read"}
    if mode in {"issue_fallback", "issue-fallback"}:
        # Creates a standalone GitHub issue.
        return {"contents": "read", "pull_requests": "read", "issues": "write"}
    if mode in {"reentry", "loop-state"}:
        return {"contents": "read", "pull_requests": "read", "issues": "read"}
    if mode in {"design_chief", "design"}:
        # Upserts the design-summary sticky comment on the PR (same PR-comment rule).
        return {"contents": "read", "pull_requests": "write", "issues": "read"}
    if mode in {"resolve_gate", "resolve"}:
        return {"contents": "read", "pull_requests": "write", "issues": "write"}
    if mode in {"review", "techlead", "comments", "write"}:
        return {"contents": "read", "pull_requests": "write", "issues": "write"}
    return {"contents": "read", "pull_requests": "write", "issues": "write"}


def assert_installation_token(token: str | None, required: dict[str, str] | None = None) -> dict[str, Any]:
    """Verify that a token is a GitHub App installation token.

    This proves token type by calling GitHub with the presented token. Repository
    scope and required permission checks are handled by
    assert_installation_token_for_repo_and_permissions().
    """
    if not token:
        raise ValidationError("GitHub App installation token is required for actual write stages")
    result = rest_request("GET", f"{_api_root()}/installation/repositories", token)
    if not isinstance(result, dict) or "repositories" not in result:
        raise ValidationError("token did not validate as a GitHub App installation token")
    req = required or {}
    if not isinstance(req, dict):
        raise ValidationError("required permissions must be a dict")
    permission_maps: list[dict[str, Any]] = []
    if isinstance(result.get("permissions"), dict):
        permission_maps.append(result["permissions"])
    permission_maps.extend(_declared_permission_maps_from_env())
    satisfied = _validate_required_permission_maps(permission_maps, req, strict=False)
    return {
        "installation_token": True,
        "repository_count": len(result.get("repositories") or []),
        "required_permissions": req,
        "permission_preflight": satisfied,
    }


def assert_installation_token_for_repo_and_permissions(
    token: str | None,
    owner: str,
    repo: str,
    required: dict[str, str] | None = None,
) -> dict[str, Any]:
    """Verify installation-token type, target repo scope, and required permissions.

    The check uses active GitHub API calls for token type and repository scope,
    then validates permissions from all trustworthy maps available: the current
    installation repository listing, the target repository object, and the
    token-creation permissions JSON threaded by the trusted workflow step.
    """
    base = assert_installation_token(token, required)
    if not owner or not repo:
        raise ValidationError("owner and repo are required for repository-scoped token validation")
    repo_listing = rest_request("GET", f"{_api_root()}/installation/repositories?per_page=100", token)
    repositories = repo_listing.get("repositories", []) if isinstance(repo_listing, dict) else []
    full_name = f"{owner}/{repo}".lower()
    matched = None
    for item in repositories:
        if not isinstance(item, dict):
            continue
        if str(item.get("full_name") or "").lower() == full_name:
            matched = item
            break
    if matched is None:
        raise ValidationError(f"GitHub App installation token is not scoped to repository {owner}/{repo}")

    repo_obj = rest_request("GET", github_api_url(owner, repo, ""), token)
    permission_maps: list[dict[str, Any]] = []
    for source in (repo_listing, matched, repo_obj):
        if isinstance(source, dict) and isinstance(source.get("permissions"), dict):
            permission_maps.append(source["permissions"])
    permission_maps.extend(_declared_permission_maps_from_env())
    satisfied = _validate_required_permission_maps(permission_maps, required or {}, strict=True)
    return {
        **base,
        "owner": owner,
        "repo": repo,
        "repository_scoped": True,
        "permission_preflight": satisfied,
    }


def assert_installation_token_for_repo(token: str | None, owner: str, repo: str, required: dict[str, str] | None = None) -> dict[str, Any]:
    return assert_installation_token_for_repo_and_permissions(token, owner, repo, required)


# Backward-compatible name used by earlier versions.
def assert_token_permissions(token: str, required: dict[str, str]) -> None:
    assert_installation_token(token, required)
