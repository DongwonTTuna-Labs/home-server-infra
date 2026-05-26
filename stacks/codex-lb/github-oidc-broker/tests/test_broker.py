from __future__ import annotations

import hashlib
import sqlite3
from datetime import UTC, datetime, timedelta

import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import Encoding, NoEncryption, PrivateFormat
from jwt import PyJWK

from app.broker import (
    AuditStore,
    BrokerConfig,
    GithubOidcClaims,
    IssuedToken,
    OidcValidationError,
    ReplayStore,
    TokenIssuer,
    verify_github_oidc_token,
)


AUDIENCE = "https://relay-ai.dongwontuna.net/github-actions"


@pytest.fixture()
def signing_key() -> rsa.RSAPrivateKey:
    return rsa.generate_private_key(public_exponent=65537, key_size=2048)


@pytest.fixture()
def public_jwk(signing_key: rsa.RSAPrivateKey) -> PyJWK:
    jwk = jwt.algorithms.RSAAlgorithm.to_jwk(signing_key.public_key(), as_dict=True)
    jwk["kid"] = "test-key"
    jwk["use"] = "sig"
    return PyJWK.from_dict(jwk)


@pytest.fixture()
def config() -> BrokerConfig:
    return BrokerConfig(
        audience=AUDIENCE,
        allowed_owner="DongwonTTuna-Labs",
        allowed_repositories={"rs-builder-relayer-client", "polymarket-liquidity-farming-rs", "bioden"},
        allowed_workflows={".github/workflows/codex-pr-review.yml", ".github/workflows/resolve-checker.yml"},
        allowed_events={"pull_request", "issue_comment", "workflow_dispatch"},
        allowed_actors={"DongwonTTuna"},
        allowed_refs={"refs/heads/main"},
        token_ttl_seconds=3600,
    )


def make_token(
    signing_key: rsa.RSAPrivateKey,
    *,
    audience: str = AUDIENCE,
    repository: str = "DongwonTTuna-Labs/bioden",
    workflow: str = ".github/workflows/codex-pr-review.yml",
    event_name: str = "pull_request",
    actor: str = "DongwonTTuna",
    runner_environment: str = "self-hosted",
    visibility: str = "private",
    issuer: str = "https://token.actions.githubusercontent.com",
    ref: str = "refs/pull/54/merge",
    base_ref: str = "main",
    head_ref: str = "codex/use-oidc-relay-token",
    workflow_ref_ref: str | None = None,
    nbf_offset_seconds: int = -5,
    exp_offset_seconds: int = 300,
    now: datetime | None = None,
) -> str:
    now = now or datetime.now(UTC)
    private_pem = signing_key.private_bytes(
        Encoding.PEM,
        PrivateFormat.PKCS8,
        NoEncryption(),
    )
    claims = {
        "iss": issuer,
        "aud": audience,
        "sub": f"repo:{repository}:pull_request",
        "repository": repository,
        "repository_owner": repository.split("/", 1)[0],
        "repository_visibility": visibility,
        "runner_environment": runner_environment,
        "workflow_ref": f"{repository}/{workflow}@{workflow_ref_ref or ref}",
        "ref": ref,
        "base_ref": base_ref,
        "head_ref": head_ref,
        "event_name": event_name,
        "actor": actor,
        "run_id": "26446723001",
        "run_attempt": "1",
        "iat": int(now.timestamp()),
        "nbf": int((now + timedelta(seconds=nbf_offset_seconds)).timestamp()),
        "exp": int((now + timedelta(seconds=exp_offset_seconds)).timestamp()),
        "jti": "jwt-test-id",
    }
    return jwt.encode(claims, private_pem, algorithm="RS256", headers={"kid": "test-key"})


def init_codex_lb_schema(path: str) -> None:
    con = sqlite3.connect(path)
    con.execute(
        """
        create table api_keys (
            id varchar primary key,
            name varchar not null,
            key_hash varchar not null unique,
            key_prefix varchar not null,
            allowed_models text,
            expires_at datetime,
            is_active boolean not null,
            created_at datetime not null default current_timestamp,
            last_used_at datetime,
            enforced_model varchar,
            enforced_reasoning_effort varchar,
            enforced_service_tier varchar,
            account_assignment_scope_enabled boolean not null default 0,
            apply_to_codex_model boolean not null default 0
        )
        """
    )
    con.commit()
    con.close()


def test_verifies_valid_github_oidc_token(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
) -> None:
    claims = verify_github_oidc_token(make_token(signing_key), public_jwk, config)

    assert claims.repository == "DongwonTTuna-Labs/bioden"
    assert claims.repository_name == "bioden"
    assert claims.workflow_file == ".github/workflows/codex-pr-review.yml"
    assert claims.run_id == "26446723001"
    assert claims.run_attempt == "1"


@pytest.mark.parametrize("event_name", ["issue_comment", "workflow_dispatch"])
def test_verifies_main_ref_events(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
    event_name: str,
) -> None:
    token = make_token(signing_key, event_name=event_name, ref="refs/heads/main")

    claims = verify_github_oidc_token(token, public_jwk, config)

    assert claims.event_name == event_name


@pytest.mark.parametrize(
    ("overrides", "message"),
    [
        ({"audience": "wrong-audience"}, "Audience"),
        ({"repository": "OtherOrg/bioden"}, "repository_owner"),
        ({"repository": "DongwonTTuna-Labs/unknown"}, "repository"),
        ({"issuer": "https://example.invalid"}, "Invalid issuer"),
        ({"workflow": ".github/workflows/deploy.yml"}, "workflow"),
        ({"ref": "refs/heads/codex/use-oidc-relay-token"}, "ref"),
        (
            {
                "event_name": "issue_comment",
                "ref": "refs/heads/main",
                "workflow_ref_ref": "refs/heads/codex/use-oidc-relay-token",
            },
            "workflow_ref ref",
        ),
        ({"base_ref": "develop"}, "base_ref"),
        ({"event_name": "push"}, "event_name"),
        ({"runner_environment": "github-hosted"}, "runner_environment"),
        ({"visibility": "public"}, "repository_visibility"),
        ({"actor": "mallory"}, "actor"),
    ],
)
def test_rejects_untrusted_claims(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
    overrides: dict[str, str],
    message: str,
) -> None:
    with pytest.raises(OidcValidationError, match=message):
        verify_github_oidc_token(make_token(signing_key, **overrides), public_jwk, config)


@pytest.mark.parametrize(
    ("overrides", "message"),
    [
        ({"exp_offset_seconds": -60}, "expired"),
        ({"nbf_offset_seconds": 60}, "not yet valid"),
    ],
)
def test_rejects_temporally_invalid_jwt(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
    overrides: dict[str, int],
    message: str,
) -> None:
    with pytest.raises(OidcValidationError, match=message):
        verify_github_oidc_token(make_token(signing_key, **overrides), public_jwk, config)


def test_replay_store_rejects_same_jwt_twice(tmp_path) -> None:
    store = ReplayStore(str(tmp_path / "broker.db"))
    token_hash = hashlib.sha256(b"jwt").hexdigest()

    assert store.record_once(token_hash, expires_at=datetime.now(UTC) + timedelta(minutes=5))
    assert not store.record_once(token_hash, expires_at=datetime.now(UTC) + timedelta(minutes=5))


def test_audit_store_records_non_secret_exchange_metadata(tmp_path) -> None:
    store = AuditStore(str(tmp_path / "broker.db"))
    claims = GithubOidcClaims(
        repository="DongwonTTuna-Labs/bioden",
        repository_name="bioden",
        workflow_file=".github/workflows/codex-pr-review.yml",
        event_name="pull_request",
        actor="DongwonTTuna",
        run_id="26446723001",
        run_attempt="1",
        expires_at=datetime.now(UTC) + timedelta(minutes=5),
    )
    issued = IssuedToken(
        relay_token="sk-clb-test-secret-value",
        expires_at=datetime.now(UTC) + timedelta(hours=1),
        api_key_id="api-key-id",
    )

    store.record_exchange(claims=claims, issued=issued)

    con = sqlite3.connect(str(tmp_path / "broker.db"))
    row = con.execute(
        """
        select repository, workflow_file, event_name, actor, run_id, run_attempt,
               api_key_id, api_key_prefix, expires_at
        from exchange_audit
        """
    ).fetchone()

    assert row[0] == "DongwonTTuna-Labs/bioden"
    assert row[1] == ".github/workflows/codex-pr-review.yml"
    assert row[2] == "pull_request"
    assert row[3] == "DongwonTTuna"
    assert row[4] == "26446723001"
    assert row[5] == "1"
    assert row[6] == "api-key-id"
    assert row[7] == "sk-clb-test-sec"
    assert row[8] is not None
    assert "secret-value" not in repr(row)


def test_token_issuer_inserts_short_lived_codex_lb_api_key(tmp_path, config: BrokerConfig) -> None:
    db_path = tmp_path / "store.db"
    init_codex_lb_schema(str(db_path))
    issuer = TokenIssuer(str(db_path), config)
    issued = issuer.issue(
        repository_name="bioden",
        workflow_file=".github/workflows/codex-pr-review.yml",
        run_id="26446723001",
        run_attempt="1",
    )

    assert issued.relay_token.startswith("sk-clb-")
    assert issued.api_key_id
    assert issued.expires_at > datetime.now(UTC) + timedelta(minutes=59)

    con = sqlite3.connect(str(db_path))
    row = con.execute(
        "select name, key_hash, key_prefix, expires_at, is_active, allowed_models from api_keys where id=?",
        (issued.api_key_id,),
    ).fetchone()

    assert row[0] == "gha:bioden:codex-pr-review.yml:26446723001:1"
    assert row[1] == hashlib.sha256(issued.relay_token.encode("utf-8")).hexdigest()
    assert row[2] == issued.relay_token[:15]
    assert row[3] is not None
    assert row[4] == 1
    assert row[5] is None
