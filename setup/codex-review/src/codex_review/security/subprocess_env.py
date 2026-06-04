"""Subprocess environment hardening helpers.

Any subprocess that touches PR worktrees or git state should avoid inheriting
GitHub App tokens, OIDC request tokens, runtime credentials, or other secrets.
Push authentication is passed explicitly through a temporary remote URL in the
single push operation, not through inherited process environment.
"""
from __future__ import annotations

import os

SENSITIVE_ENV_PREFIXES = (
    "CODEX_REVIEW_GITHUB_APP_",
    "GITHUB_APP_",
)
SENSITIVE_ENV_NAMES = {
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_TOKEN",
    "ACTIONS_ID_TOKEN_REQUEST_URL",
    "ACTIONS_RUNTIME_TOKEN",
    "ACTIONS_CACHE_URL",
    "ACTIONS_RESULTS_URL",
    "CODEX_REVIEW_GITHUB_APP_ID",
    "CODEX_REVIEW_GITHUB_APP_PRIVATE_KEY",
    "CODEX_REVIEW_APP_ID",
    "CODEX_REVIEW_APP_PRIVATE_KEY",
    "GITHUB_APP_ID",
    "GITHUB_APP_PRIVATE_KEY",
    "CODEX_APP_ID",
    "CODEX_APP_PRIVATE_KEY",
    "CODEX_REVIEW_APP_TOKEN_PERMISSIONS",
    "CODEX_REVIEW_APP_TOKEN_PERMISSIONS_JSON",
}


def sanitized_env(extra: dict[str, str] | None = None) -> dict[str, str]:
    env = dict(os.environ)
    for name in list(env):
        upper = name.upper()
        if (
            name in SENSITIVE_ENV_NAMES
            or any(name.startswith(prefix) for prefix in SENSITIVE_ENV_PREFIXES)
            or "TOKEN" in upper
            or "SECRET" in upper
            or "PRIVATE_KEY" in upper
            or "PASSWORD" in upper
        ):
            env.pop(name, None)
    if extra:
        env.update({str(k): str(v) for k, v in extra.items()})
    return env
