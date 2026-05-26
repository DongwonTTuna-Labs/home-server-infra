from __future__ import annotations

import hashlib
import os
import secrets
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Iterable

import jwt
from fastapi import FastAPI, HTTPException
from jwt import InvalidTokenError, PyJWK, PyJWKClient, PyJWKClientError
from pydantic import BaseModel, Field

GITHUB_OIDC_ISSUER = "https://token.actions.githubusercontent.com"
GITHUB_OIDC_JWKS_URL = "https://token.actions.githubusercontent.com/.well-known/jwks"
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
EXPECTED_API_KEY_COLUMNS = {
    "id",
    "name",
    "key_hash",
    "key_prefix",
    "allowed_models",
    "expires_at",
    "is_active",
    "created_at",
    "last_used_at",
    "enforced_model",
    "enforced_reasoning_effort",
    "enforced_service_tier",
    "account_assignment_scope_enabled",
    "apply_to_codex_model",
}


class ExchangeRequest(BaseModel):
    token: str = Field(min_length=1)


class ExchangeResponse(BaseModel):
    token_type: str = "Bearer"
    relay_token: str
    expires_at: datetime
    api_key_id: str


class OidcValidationError(ValueError):
    pass


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
    codex_lb_db_path: str = "/var/lib/codex-lb/store.db"
    broker_db_path: str = "/var/lib/github-oidc-broker/broker.db"

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
            codex_lb_db_path=os.getenv("BROKER_CODEX_LB_DB_PATH", "/var/lib/codex-lb/store.db"),
            broker_db_path=os.getenv("BROKER_DB_PATH", "/var/lib/github-oidc-broker/broker.db"),
        )


@dataclass(frozen=True, slots=True)
class GithubOidcClaims:
    repository: str
    repository_name: str
    workflow_file: str
    event_name: str
    actor: str
    run_id: str
    run_attempt: str
    expires_at: datetime


@dataclass(frozen=True, slots=True)
class IssuedToken:
    relay_token: str
    expires_at: datetime
    api_key_id: str


def _csv_env(name: str, default: Iterable[str]) -> frozenset[str]:
    raw = os.getenv(name)
    if raw is None:
        return frozenset(default)
    return frozenset(item.strip() for item in raw.split(",") if item.strip())


def _utc_now() -> datetime:
    return datetime.now(UTC)


def _sqlite_datetime(value: datetime) -> str:
    return value.astimezone(UTC).replace(tzinfo=None).strftime("%Y-%m-%d %H:%M:%S.%f")


def _parse_expires_at(claims: dict[str, object]) -> datetime:
    exp = claims.get("exp")
    if not isinstance(exp, int):
        raise OidcValidationError("exp claim is required")
    return datetime.fromtimestamp(exp, UTC)


def _extract_workflow_file(repository: str, workflow_ref: object) -> str:
    if not isinstance(workflow_ref, str):
        raise OidcValidationError("workflow_ref claim is required")
    prefix = f"{repository}/"
    if not workflow_ref.startswith(prefix):
        raise OidcValidationError("workflow_ref repository mismatch")
    workflow_spec = workflow_ref[len(prefix) :]
    if "@" not in workflow_spec:
        raise OidcValidationError("workflow_ref ref is required")
    workflow_path, workflow_ref_ref = workflow_spec.rsplit("@", 1)
    if not workflow_path:
        raise OidcValidationError("workflow_ref path is required")
    if not workflow_ref_ref:
        raise OidcValidationError("workflow_ref ref is required")
    return workflow_path


def _extract_workflow_ref_ref(repository: str, workflow_ref: object) -> str:
    if not isinstance(workflow_ref, str):
        raise OidcValidationError("workflow_ref claim is required")
    prefix = f"{repository}/"
    if not workflow_ref.startswith(prefix):
        raise OidcValidationError("workflow_ref repository mismatch")
    workflow_spec = workflow_ref[len(prefix) :]
    if "@" not in workflow_spec:
        raise OidcValidationError("workflow_ref ref is required")
    workflow_ref_ref = workflow_spec.rsplit("@", 1)[1]
    if not workflow_ref_ref:
        raise OidcValidationError("workflow_ref ref is required")
    return workflow_ref_ref


def _allowed_base_refs(config: BrokerConfig) -> frozenset[str]:
    return frozenset(ref.removeprefix("refs/heads/") for ref in config.allowed_refs if ref.startswith("refs/heads/"))


def _assert_trusted_ref(claims: dict[str, object], workflow_ref: object, repository: str, config: BrokerConfig) -> None:
    ref = _require_claim(claims, "ref")
    workflow_ref_ref = _extract_workflow_ref_ref(repository, workflow_ref)
    event_name = _require_claim(claims, "event_name")
    if event_name == "pull_request":
        if not ref.startswith("refs/pull/") or not ref.endswith("/merge"):
            raise OidcValidationError("pull_request ref is not allowed")
        base_ref = _require_claim(claims, "base_ref")
        if base_ref not in _allowed_base_refs(config):
            raise OidcValidationError("base_ref is not allowed")
        return
    if ref not in config.allowed_refs:
        raise OidcValidationError("ref is not allowed")
    if workflow_ref_ref != ref:
        raise OidcValidationError("workflow_ref ref does not match ref claim")


def verify_github_oidc_token(token: str, signing_key: PyJWK, config: BrokerConfig) -> GithubOidcClaims:
    try:
        claims = jwt.decode(
            token,
            signing_key.key,
            algorithms=["RS256"],
            audience=config.audience,
            issuer=GITHUB_OIDC_ISSUER,
            options={"require": ["exp", "iat", "nbf", "iss", "aud"]},
        )
    except InvalidTokenError as exc:
        raise OidcValidationError(str(exc)) from exc

    repository = _require_claim(claims, "repository")
    owner = _require_claim(claims, "repository_owner")
    if owner != config.allowed_owner:
        raise OidcValidationError("repository_owner is not allowed")
    if not repository.startswith(f"{config.allowed_owner}/"):
        raise OidcValidationError("repository owner prefix is not allowed")
    repository_name = repository.split("/", 1)[1]
    if repository_name not in config.allowed_repositories:
        raise OidcValidationError("repository is not allowed")

    if _require_claim(claims, "repository_visibility") != "private":
        raise OidcValidationError("repository_visibility is not allowed")
    if _require_claim(claims, "runner_environment") != "self-hosted":
        raise OidcValidationError("runner_environment is not allowed")

    workflow_ref = claims.get("workflow_ref")
    workflow_file = _extract_workflow_file(repository, workflow_ref)
    if workflow_file not in config.allowed_workflows:
        raise OidcValidationError("workflow is not allowed")

    event_name = _require_claim(claims, "event_name")
    if event_name not in config.allowed_events:
        raise OidcValidationError("event_name is not allowed")
    _assert_trusted_ref(claims, workflow_ref, repository, config)

    actor = _require_claim(claims, "actor")
    if actor not in config.allowed_actors:
        raise OidcValidationError("actor is not allowed")

    return GithubOidcClaims(
        repository=repository,
        repository_name=repository_name,
        workflow_file=workflow_file,
        event_name=event_name,
        actor=actor,
        run_id=_require_claim(claims, "run_id"),
        run_attempt=_require_claim(claims, "run_attempt"),
        expires_at=_parse_expires_at(claims),
    )


def _require_claim(claims: dict[str, object], name: str) -> str:
    value = claims.get(name)
    if not isinstance(value, str) or not value:
        raise OidcValidationError(f"{name} claim is required")
    return value


class ReplayStore:
    def __init__(self, db_path: str) -> None:
        self.db_path = db_path

    def _ensure_parent(self) -> None:
        Path(self.db_path).parent.mkdir(parents=True, exist_ok=True)

    def _connect(self) -> sqlite3.Connection:
        con = sqlite3.connect(self.db_path, timeout=30)
        con.execute("pragma journal_mode=wal")
        con.execute("pragma busy_timeout=30000")
        return con

    def _init_schema(self) -> None:
        self._ensure_parent()
        with self._connect() as con:
            con.execute(
                """
                create table if not exists oidc_replays (
                    token_hash text primary key,
                    expires_at text not null,
                    created_at text not null default current_timestamp
                )
                """
            )

    def record_once(self, token_hash: str, *, expires_at: datetime) -> bool:
        self._init_schema()
        now = _utc_now()
        with self._connect() as con:
            con.execute("delete from oidc_replays where expires_at <= ?", (_sqlite_datetime(now),))
            try:
                con.execute(
                    "insert into oidc_replays (token_hash, expires_at) values (?, ?)",
                    (token_hash, _sqlite_datetime(expires_at)),
                )
            except sqlite3.IntegrityError:
                return False
        return True


class AuditStore:
    def __init__(self, db_path: str) -> None:
        self.db_path = db_path

    def _ensure_parent(self) -> None:
        Path(self.db_path).parent.mkdir(parents=True, exist_ok=True)

    def _connect(self) -> sqlite3.Connection:
        con = sqlite3.connect(self.db_path, timeout=30)
        con.execute("pragma journal_mode=wal")
        con.execute("pragma busy_timeout=30000")
        return con

    def _init_schema(self) -> None:
        self._ensure_parent()
        with self._connect() as con:
            con.execute(
                """
                create table if not exists exchange_audit (
                    id text primary key,
                    repository text not null,
                    workflow_file text not null,
                    event_name text not null,
                    actor text not null,
                    run_id text not null,
                    run_attempt text not null,
                    api_key_id text not null,
                    api_key_prefix text not null,
                    expires_at text not null,
                    created_at text not null default current_timestamp
                )
                """
            )

    def record_exchange(self, *, claims: GithubOidcClaims, issued: IssuedToken) -> None:
        self._init_schema()
        with self._connect() as con:
            con.execute(
                """
                insert into exchange_audit (
                    id,
                    repository,
                    workflow_file,
                    event_name,
                    actor,
                    run_id,
                    run_attempt,
                    api_key_id,
                    api_key_prefix,
                    expires_at
                ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    str(uuid.uuid4()),
                    claims.repository,
                    claims.workflow_file,
                    claims.event_name,
                    claims.actor,
                    claims.run_id,
                    claims.run_attempt,
                    issued.api_key_id,
                    issued.relay_token[:15],
                    _sqlite_datetime(issued.expires_at),
                ),
            )


class TokenIssuer:
    def __init__(self, db_path: str, config: BrokerConfig) -> None:
        self.db_path = db_path
        self.config = config

    def issue(
        self,
        *,
        repository_name: str,
        workflow_file: str,
        run_id: str,
        run_attempt: str,
    ) -> IssuedToken:
        expires_at = _utc_now() + timedelta(seconds=self.config.token_ttl_seconds)
        relay_token = f"sk-clb-{secrets.token_urlsafe(32)}"
        api_key_id = str(uuid.uuid4())
        workflow_name = Path(workflow_file).name
        name = f"gha:{repository_name}:{workflow_name}:{run_id}:{run_attempt}"
        with self._connect() as con:
            self._assert_schema(con)
            con.execute(
                """
                insert into api_keys (
                    id,
                    name,
                    key_hash,
                    key_prefix,
                    allowed_models,
                    expires_at,
                    is_active,
                    created_at,
                    last_used_at,
                    enforced_model,
                    enforced_reasoning_effort,
                    enforced_service_tier,
                    account_assignment_scope_enabled,
                    apply_to_codex_model
                ) values (?, ?, ?, ?, null, ?, 1, ?, null, null, null, null, 0, 0)
                """,
                (
                    api_key_id,
                    name,
                    hashlib.sha256(relay_token.encode("utf-8")).hexdigest(),
                    relay_token[:15],
                    _sqlite_datetime(expires_at),
                    _sqlite_datetime(_utc_now()),
                ),
            )
        return IssuedToken(relay_token=relay_token, expires_at=expires_at, api_key_id=api_key_id)

    def _connect(self) -> sqlite3.Connection:
        con = sqlite3.connect(self.db_path, timeout=30)
        con.execute("pragma busy_timeout=30000")
        con.execute("pragma foreign_keys=on")
        return con

    def _assert_schema(self, con: sqlite3.Connection) -> None:
        rows = con.execute("pragma table_info(api_keys)").fetchall()
        columns = {row[1] for row in rows}
        missing = sorted(EXPECTED_API_KEY_COLUMNS - columns)
        if missing:
            raise RuntimeError(f"codex-lb api_keys schema is missing expected columns: {', '.join(missing)}")


def create_app(config: BrokerConfig | None = None) -> FastAPI:
    config = config or BrokerConfig.from_env()
    jwks_client = PyJWKClient(GITHUB_OIDC_JWKS_URL)
    replay_store = ReplayStore(config.broker_db_path)
    audit_store = AuditStore(config.broker_db_path)
    issuer = TokenIssuer(config.codex_lb_db_path, config)
    app = FastAPI(title="GitHub OIDC broker for codex-lb")

    @app.get("/oidc/health")
    def health() -> dict[str, str]:
        return {"status": "ok"}

    @app.post("/oidc/exchange", response_model=ExchangeResponse)
    def exchange(payload: ExchangeRequest) -> ExchangeResponse:
        token_hash = hashlib.sha256(payload.token.encode("utf-8")).hexdigest()
        try:
            signing_key = jwks_client.get_signing_key_from_jwt(payload.token)
        except (InvalidTokenError, PyJWKClientError) as exc:
            raise HTTPException(status_code=403, detail="Invalid OIDC token") from exc

        try:
            claims = verify_github_oidc_token(payload.token, signing_key, config)
            if not replay_store.record_once(token_hash, expires_at=claims.expires_at):
                raise HTTPException(status_code=409, detail="OIDC token has already been exchanged")
            issued = issuer.issue(
                repository_name=claims.repository_name,
                workflow_file=claims.workflow_file,
                run_id=claims.run_id,
                run_attempt=claims.run_attempt,
            )
            audit_store.record_exchange(claims=claims, issued=issued)
        except HTTPException:
            raise
        except OidcValidationError as exc:
            raise HTTPException(status_code=403, detail=str(exc)) from exc
        except Exception as exc:
            raise HTTPException(status_code=500, detail="Unable to mint relay token") from exc
        return ExchangeResponse(
            relay_token=issued.relay_token,
            expires_at=issued.expires_at,
            api_key_id=issued.api_key_id,
        )

    return app


app = create_app()
