from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from decimal import Decimal, InvalidOperation, ROUND_HALF_UP
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
DEFAULT_ALLOWED_EVENTS_BY_WORKFLOW = {
    ".github/workflows/codex-pr-review.yml": frozenset({"pull_request_target", "issue_comment"}),
    ".github/workflows/resolve-checker.yml": frozenset({"workflow_run", "workflow_dispatch"}),
}
DEFAULT_ALLOWED_ACTORS = frozenset({"DongwonTTuna"})
DEFAULT_ALLOWED_REFS = frozenset({"refs/heads/main"})
DEFAULT_CODEX_LB_BASE_URL = "http://codex-lb:2455"
DEFAULT_CODEX_LB_ENCRYPTION_KEY_PATH = "/var/lib/codex-lb/encryption.key"
DEFAULT_DASHBOARD_SESSION_TTL_SECONDS = 60
DEFAULT_HTTP_TIMEOUT_SECONDS = 10.0
DEFAULT_API_KEY_COST_LIMIT_USD = "50"
DEFAULT_API_KEY_COST_LIMIT_WINDOW = "weekly"
MICRODOLLARS_PER_USD = Decimal("1000000")
API_KEY_COST_LIMIT_WINDOWS = frozenset({"daily", "weekly", "monthly", "5h", "7d"})


def _csv_env(name: str, default: Iterable[str]) -> frozenset[str]:
    raw = os.getenv(name)
    if raw is None:
        return frozenset(default)
    return frozenset(item.strip() for item in raw.split(",") if item.strip())


def _workflow_events_env(name: str, default: dict[str, frozenset[str]]) -> dict[str, frozenset[str]]:
    if os.getenv("BROKER_ALLOWED_EVENTS") is not None:
        raise ValueError("BROKER_ALLOWED_EVENTS is no longer supported; use BROKER_ALLOWED_EVENTS_BY_WORKFLOW")
    raw = os.getenv(name)
    if raw is None:
        return {workflow: frozenset(events) for workflow, events in default.items()}
    try:
        parsed = json.loads(raw)
    except json.JSONDecodeError as exc:
        raise ValueError(f"{name} must be a JSON object mapping workflow files to event lists") from exc
    if not isinstance(parsed, dict):
        raise ValueError(f"{name} must be a JSON object mapping workflow files to event lists")
    result: dict[str, frozenset[str]] = {}
    for workflow, events in parsed.items():
        if not isinstance(workflow, str) or not workflow.strip():
            raise ValueError(f"{name} workflow keys must be non-empty strings")
        if not isinstance(events, list) or not events:
            raise ValueError(f"{name}[{workflow!r}] must be a non-empty event list")
        normalized = frozenset(str(event).strip() for event in events if str(event).strip())
        if len(normalized) != len(events):
            raise ValueError(f"{name}[{workflow!r}] contains an empty event")
        result[workflow.strip()] = normalized
    return result


def _cost_limit_microdollars_env(name: str, default: str) -> int:
    raw = os.getenv(name, default).strip()
    try:
        usd = Decimal(raw)
    except InvalidOperation as exc:
        raise ValueError(f"{name} must be a decimal USD amount") from exc
    microdollars = int((usd * MICRODOLLARS_PER_USD).to_integral_value(rounding=ROUND_HALF_UP))
    if microdollars < 1:
        raise ValueError(f"{name} must be greater than zero")
    return microdollars


def _cost_limit_window_env(name: str, default: str) -> str:
    window = os.getenv(name, default).strip()
    if window not in API_KEY_COST_LIMIT_WINDOWS:
        raise ValueError(f"{name} must be one of {', '.join(sorted(API_KEY_COST_LIMIT_WINDOWS))}")
    return window


@dataclass(frozen=True, slots=True)
class BrokerConfig:
    audience: str = DEFAULT_AUDIENCE
    allowed_owner: str = DEFAULT_ALLOWED_OWNER
    allowed_repositories: set[str] | frozenset[str] = DEFAULT_ALLOWED_REPOSITORIES
    allowed_workflows: set[str] | frozenset[str] = DEFAULT_ALLOWED_WORKFLOWS
    allowed_events_by_workflow: dict[str, set[str] | frozenset[str]] = field(
        default_factory=lambda: {
            workflow: frozenset(events) for workflow, events in DEFAULT_ALLOWED_EVENTS_BY_WORKFLOW.items()
        }
    )
    allowed_actors: set[str] | frozenset[str] = DEFAULT_ALLOWED_ACTORS
    allowed_refs: set[str] | frozenset[str] = DEFAULT_ALLOWED_REFS
    token_ttl_seconds: int = 3600
    broker_db_path: str = "/var/lib/github-oidc-broker/broker.db"
    codex_lb_base_url: str = DEFAULT_CODEX_LB_BASE_URL
    codex_lb_encryption_key_path: str = DEFAULT_CODEX_LB_ENCRYPTION_KEY_PATH
    dashboard_session_ttl_seconds: int = DEFAULT_DASHBOARD_SESSION_TTL_SECONDS
    http_timeout_seconds: float = DEFAULT_HTTP_TIMEOUT_SECONDS
    api_key_cost_limit_microdollars: int = 50_000_000
    api_key_cost_limit_window: str = DEFAULT_API_KEY_COST_LIMIT_WINDOW

    @classmethod
    def from_env(cls) -> "BrokerConfig":
        return cls(
            audience=os.getenv("BROKER_AUDIENCE", DEFAULT_AUDIENCE),
            allowed_owner=os.getenv("BROKER_ALLOWED_OWNER", DEFAULT_ALLOWED_OWNER),
            allowed_repositories=_csv_env("BROKER_ALLOWED_REPOSITORIES", DEFAULT_ALLOWED_REPOSITORIES),
            allowed_workflows=_csv_env("BROKER_ALLOWED_WORKFLOWS", DEFAULT_ALLOWED_WORKFLOWS),
            allowed_events_by_workflow=_workflow_events_env(
                "BROKER_ALLOWED_EVENTS_BY_WORKFLOW",
                DEFAULT_ALLOWED_EVENTS_BY_WORKFLOW,
            ),
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
            api_key_cost_limit_microdollars=_cost_limit_microdollars_env(
                "BROKER_API_KEY_COST_LIMIT_USD",
                DEFAULT_API_KEY_COST_LIMIT_USD,
            ),
            api_key_cost_limit_window=_cost_limit_window_env(
                "BROKER_API_KEY_COST_LIMIT_WINDOW",
                DEFAULT_API_KEY_COST_LIMIT_WINDOW,
            ),
        )
