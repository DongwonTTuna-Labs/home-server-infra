from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Iterable

DEFAULT_AUDIENCE = "https://relay-ai.dongwontuna.net/github-actions"
DEFAULT_ALLOWED_OWNER = "DongwonTTuna-Labs"
DEFAULT_ALLOWED_REPOSITORIES = frozenset(
    {
        "rs-builder-relayer-client",
        "polymarket-liquidity-farming-rs",
        "bioden",
    }
)
DEFAULT_ALLOWED_WORKFLOWS = frozenset(
    {
        ".github/workflows/codex-pr-review.yml",
        ".github/workflows/resolve-checker.yml",
    }
)
DEFAULT_ALLOWED_EVENTS = frozenset({"pull_request", "issue_comment", "workflow_dispatch"})
DEFAULT_ALLOWED_ACTORS = frozenset({"DongwonTTuna"})
DEFAULT_ALLOWED_REFS = frozenset({"refs/heads/main"})
DEFAULT_CODEX_LB_BASE_URL = "http://codex-lb:2455"
DEFAULT_CODEX_LB_ENCRYPTION_KEY_PATH = "/var/lib/codex-lb/encryption.key"
DEFAULT_DASHBOARD_SESSION_TTL_SECONDS = 60
DEFAULT_HTTP_TIMEOUT_SECONDS = 10.0


def _csv_env(name: str, default: Iterable[str]) -> frozenset[str]:
    raw = os.getenv(name)
    if raw is None:
        return frozenset(default)
    return frozenset(item.strip() for item in raw.split(",") if item.strip())


@dataclass(frozen=True, slots=True)
class BrokerConfig:
    audience: str = DEFAULT_AUDIENCE
    allowed_owner: str = DEFAULT_ALLOWED_OWNER
    allowed_repositories: set[str] | frozenset[str] = DEFAULT_ALLOWED_REPOSITORIES
    allowed_workflows: set[str] | frozenset[str] = DEFAULT_ALLOWED_WORKFLOWS
    allowed_events: set[str] | frozenset[str] = DEFAULT_ALLOWED_EVENTS
    allowed_actors: set[str] | frozenset[str] = DEFAULT_ALLOWED_ACTORS
    allowed_refs: set[str] | frozenset[str] = DEFAULT_ALLOWED_REFS
    token_ttl_seconds: int = 3600
    broker_db_path: str = "/var/lib/github-oidc-broker/broker.db"
    codex_lb_base_url: str = DEFAULT_CODEX_LB_BASE_URL
    codex_lb_encryption_key_path: str = DEFAULT_CODEX_LB_ENCRYPTION_KEY_PATH
    dashboard_session_ttl_seconds: int = DEFAULT_DASHBOARD_SESSION_TTL_SECONDS
    http_timeout_seconds: float = DEFAULT_HTTP_TIMEOUT_SECONDS

    @classmethod
    def from_env(cls) -> "BrokerConfig":
        return cls(
            audience=os.getenv("BROKER_AUDIENCE", DEFAULT_AUDIENCE),
            allowed_owner=os.getenv("BROKER_ALLOWED_OWNER", DEFAULT_ALLOWED_OWNER),
            allowed_repositories=_csv_env("BROKER_ALLOWED_REPOSITORIES", DEFAULT_ALLOWED_REPOSITORIES),
            allowed_workflows=_csv_env("BROKER_ALLOWED_WORKFLOWS", DEFAULT_ALLOWED_WORKFLOWS),
            allowed_events=_csv_env("BROKER_ALLOWED_EVENTS", DEFAULT_ALLOWED_EVENTS),
            allowed_actors=_csv_env("BROKER_ALLOWED_ACTORS", DEFAULT_ALLOWED_ACTORS),
            allowed_refs=_csv_env("BROKER_ALLOWED_REFS", DEFAULT_ALLOWED_REFS),
            token_ttl_seconds=int(os.getenv("BROKER_TOKEN_TTL_SECONDS", "3600")),
            broker_db_path=os.getenv("BROKER_DB_PATH", "/var/lib/github-oidc-broker/broker.db"),
            codex_lb_base_url=os.getenv("BROKER_CODEX_LB_BASE_URL", DEFAULT_CODEX_LB_BASE_URL),
            codex_lb_encryption_key_path=os.getenv(
                "BROKER_CODEX_LB_ENCRYPTION_KEY_PATH",
                DEFAULT_CODEX_LB_ENCRYPTION_KEY_PATH,
            ),
            dashboard_session_ttl_seconds=int(
                os.getenv("BROKER_DASHBOARD_SESSION_TTL_SECONDS", str(DEFAULT_DASHBOARD_SESSION_TTL_SECONDS))
            ),
            http_timeout_seconds=float(os.getenv("BROKER_HTTP_TIMEOUT_SECONDS", str(DEFAULT_HTTP_TIMEOUT_SECONDS))),
        )
