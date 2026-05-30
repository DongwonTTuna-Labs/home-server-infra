from __future__ import annotations

import hashlib
from datetime import UTC, datetime

import jwt
from fastapi import FastAPI, HTTPException
from jwt import InvalidTokenError, PyJWK, PyJWKClient, PyJWKClientError
from pydantic import BaseModel, Field

from app.codex_lb import CodexLbDashboardClient
from app.config import BrokerConfig
from app.models import GithubOidcClaims
from app.store import AuditStore, ReplayStore

GITHUB_OIDC_ISSUER = "https://token.actions.githubusercontent.com"
GITHUB_OIDC_JWKS_URL = "https://token.actions.githubusercontent.com/.well-known/jwks"


class ExchangeRequest(BaseModel):
    token: str = Field(min_length=1)


class ExchangeResponse(BaseModel):
    token_type: str = "Bearer"
    relay_token: str
    expires_at: datetime
    api_key_id: str


class OidcValidationError(ValueError):
    pass


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


def _assert_trusted_ref(claims: dict[str, object], workflow_ref: object, repository: str, config: BrokerConfig) -> None:
    ref = _require_claim(claims, "ref")
    if ref not in config.allowed_refs:
        raise OidcValidationError("ref is not allowed")
    if not isinstance(workflow_ref, str):
        raise OidcValidationError("workflow_ref claim is required")
    prefix = f"{repository}/"
    workflow_ref_ref = workflow_ref[len(prefix) :].rsplit("@", 1)[1]
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
    _assert_trusted_ref(claims, workflow_ref, repository, config)

    event_name = _require_claim(claims, "event_name")
    allowed_events_for_workflow = config.allowed_events_by_workflow.get(workflow_file)
    if not allowed_events_for_workflow or event_name not in allowed_events_for_workflow:
        raise OidcValidationError("workflow_event_pair_denied")

    actor = _require_claim(claims, "actor")
    allowed_actors_for_event = (config.allowed_actors_by_workflow_event.get(workflow_file) or {}).get(event_name)
    if not allowed_actors_for_event or actor not in allowed_actors_for_event:
        raise OidcValidationError("workflow_event_actor_pair_denied: actor is not allowed")

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


def create_app(
    config: BrokerConfig | None = None,
    *,
    jwks_client: PyJWKClient | None = None,
    audit_store: AuditStore | None = None,
    dashboard_client: CodexLbDashboardClient | None = None,
) -> FastAPI:
    config = config or BrokerConfig.from_env()
    jwks_client = jwks_client or PyJWKClient(GITHUB_OIDC_JWKS_URL)
    replay_store = ReplayStore(config.broker_db_path)
    audit_store = audit_store or AuditStore(config.broker_db_path)
    dashboard_client = dashboard_client or CodexLbDashboardClient(config)
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
            issued = dashboard_client.create_key(
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
            raise HTTPException(status_code=500, detail="Unable to complete OIDC exchange") from exc
        return ExchangeResponse(
            relay_token=issued.relay_token,
            expires_at=issued.expires_at,
            api_key_id=issued.api_key_id,
        )

    return app


app = create_app()
