from __future__ import annotations

import hashlib
import json
import sqlite3
from dataclasses import replace
from datetime import UTC, datetime, timedelta

import httpx
import jwt
import pytest
from cryptography.fernet import Fernet
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.hazmat.primitives.serialization import Encoding, NoEncryption, PrivateFormat
from fastapi.testclient import TestClient
from jwt import PyJWK

import app.cleanup_expired_keys as cleanup_cli
from app.broker import (
    OidcValidationError,
    create_app,
    verify_github_oidc_token,
)
from app.codex_lb import (
    DASHBOARD_SESSION_COOKIE,
    CodexLbClientError,
    CodexLbDashboardClient,
    DashboardSessionCookieFactory,
)
from app.config import DEFAULT_ALLOWED_EVENTS_BY_WORKFLOW, DEFAULT_AUDIENCE, BrokerConfig
from app.models import GithubOidcClaims, IssuedToken
from app.store import AuditStore, ReplayStore

AUDIENCE = DEFAULT_AUDIENCE


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
        allowed_events_by_workflow={
            ".github/workflows/codex-pr-review.yml": {"pull_request_target", "issue_comment"},
            ".github/workflows/resolve-checker.yml": {"workflow_run", "workflow_dispatch"},
        },
        allowed_actors={"DongwonTTuna"},
        allowed_refs={"refs/heads/main"},
        token_ttl_seconds=3600,
        codex_lb_base_url="http://codex-lb.test",
    )


def test_config_parses_api_key_cost_limit_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("BROKER_API_KEY_COST_LIMIT_USD", "50")
    monkeypatch.setenv("BROKER_API_KEY_COST_LIMIT_WINDOW", "weekly")

    parsed = BrokerConfig.from_env()

    assert parsed.api_key_cost_limit_microdollars == 50_000_000
    assert parsed.api_key_cost_limit_window == "weekly"


def make_token(
    signing_key: rsa.RSAPrivateKey,
    *,
    audience: str = AUDIENCE,
    repository: str = "DongwonTTuna-Labs/bioden",
    workflow: str = ".github/workflows/codex-pr-review.yml",
    event_name: str = "pull_request_target",
    actor: str = "DongwonTTuna",
    runner_environment: str = "self-hosted",
    visibility: str = "private",
    issuer: str = "https://token.actions.githubusercontent.com",
    ref: str = "refs/heads/main",
    workflow_ref_ref: str | None = None,
    nbf_offset_seconds: int = -5,
    exp_offset_seconds: int = 300,
    now: datetime | None = None,
) -> str:
    now = now or datetime.now(UTC)
    private_pem = signing_key.private_bytes(Encoding.PEM, PrivateFormat.PKCS8, NoEncryption())
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


def broker_claims() -> GithubOidcClaims:
    return GithubOidcClaims(
        repository="DongwonTTuna-Labs/bioden",
        repository_name="bioden",
        workflow_file=".github/workflows/codex-pr-review.yml",
        event_name="pull_request_target",
        actor="DongwonTTuna",
        run_id="26446723001",
        run_attempt="1",
        expires_at=datetime.now(UTC) + timedelta(minutes=5),
    )


class StaticSessionFactory:
    def create(self) -> str:
        return "session-cookie"


class StaticJwksClient:
    def __init__(self, signing_key: PyJWK) -> None:
        self.signing_key = signing_key

    def get_signing_key_from_jwt(self, token: str) -> PyJWK:
        return self.signing_key


class RecordingDashboardClient:
    def __init__(self, *, fail: bool = False, delete_fail_ids: set[str] | None = None) -> None:
        self.fail = fail
        self.delete_fail_ids = delete_fail_ids or set()
        self.created = 0
        self.deleted: list[str] = []

    def create_key(self, **kwargs) -> IssuedToken:
        self.created += 1
        if self.fail:
            raise CodexLbClientError("create failed")
        return IssuedToken(
            relay_token="sk-clb-created",
            expires_at=datetime.now(UTC) + timedelta(hours=1),
            api_key_id="api-key-id",
        )

    def delete_key(self, api_key_id: str) -> bool:
        self.deleted.append(api_key_id)
        if api_key_id in self.delete_fail_ids:
            raise CodexLbClientError("delete failed")
        return api_key_id != "already-gone"


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


def test_default_event_allowlist_is_workflow_specific() -> None:
    assert DEFAULT_ALLOWED_EVENTS_BY_WORKFLOW == {
        ".github/workflows/codex-pr-review.yml": frozenset({"pull_request_target", "issue_comment"}),
        ".github/workflows/resolve-checker.yml": frozenset({"workflow_run", "workflow_dispatch"}),
    }


def test_verifies_trusted_pull_request_target_token(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
) -> None:
    claims = verify_github_oidc_token(
        make_token(signing_key, event_name="pull_request_target"),
        public_jwk,
        config,
    )

    assert claims.event_name == "pull_request_target"
    assert claims.workflow_file == ".github/workflows/codex-pr-review.yml"


@pytest.mark.parametrize(
    ("workflow", "event_name"),
    [
        (".github/workflows/codex-pr-review.yml", "issue_comment"),
        (".github/workflows/resolve-checker.yml", "workflow_run"),
        (".github/workflows/resolve-checker.yml", "workflow_dispatch"),
    ],
)
def test_verifies_allowed_workflow_event_pairs(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
    workflow: str,
    event_name: str,
) -> None:
    claims = verify_github_oidc_token(
        make_token(signing_key, workflow=workflow, event_name=event_name),
        public_jwk,
        config,
    )

    assert claims.workflow_file == workflow
    assert claims.event_name == event_name


@pytest.mark.parametrize(
    ("workflow", "event_name"),
    [
        (".github/workflows/codex-pr-review.yml", "workflow_run"),
        (".github/workflows/resolve-checker.yml", "pull_request_target"),
        (".github/workflows/resolve-checker.yml", "issue_comment"),
    ],
)
def test_rejects_disallowed_workflow_event_pairs(
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
    workflow: str,
    event_name: str,
) -> None:
    with pytest.raises(OidcValidationError, match="workflow_event_pair_denied"):
        verify_github_oidc_token(
            make_token(signing_key, workflow=workflow, event_name=event_name),
            public_jwk,
            config,
        )


def test_config_rejects_legacy_global_allowed_events(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("BROKER_ALLOWED_EVENTS", "workflow_run")

    with pytest.raises(ValueError, match="BROKER_ALLOWED_EVENTS is no longer supported"):
        BrokerConfig.from_env()


@pytest.mark.parametrize(
    ("overrides", "message"),
    [
        ({"audience": "wrong-audience"}, "Audience"),
        ({"repository": "OtherOrg/bioden"}, "repository_owner"),
        ({"repository": "DongwonTTuna-Labs/unknown"}, "repository"),
        ({"issuer": "https://example.invalid"}, "Invalid issuer"),
        ({"workflow": ".github/workflows/deploy.yml"}, "workflow"),
        ({"ref": "refs/heads/codex/use-oidc-relay-token"}, "ref"),
        ({"workflow_ref_ref": "refs/heads/codex/use-oidc-relay-token"}, "workflow_ref ref"),
        ({"event_name": "push"}, "workflow_event_pair_denied"),
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
    issued = IssuedToken("sk-clb-test-secret-value", datetime.now(UTC) + timedelta(hours=1), "api-key-id")

    store.record_exchange(claims=broker_claims(), issued=issued)

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
    assert row[2] == "pull_request_target"
    assert row[3] == "DongwonTTuna"
    assert row[4] == "26446723001"
    assert row[5] == "1"
    assert row[6] == "api-key-id"
    assert row[7] == "sk-clb-test-sec"
    assert row[8] is not None
    assert "secret-value" not in repr(row)


def test_dashboard_session_cookie_is_codex_lb_compatible(tmp_path) -> None:
    key_path = tmp_path / "encryption.key"
    key = Fernet.generate_key()
    key_path.write_bytes(key)

    cookie = DashboardSessionCookieFactory(encryption_key_path=str(key_path), ttl_seconds=60).create()
    payload = json.loads(Fernet(key).decrypt(cookie.encode("ascii")))

    assert payload["pw"] is True
    assert payload["tv"] is True
    assert payload["exp"] > int(datetime.now(UTC).timestamp())


def test_dashboard_client_creates_key_through_codex_lb_api(config: BrokerConfig) -> None:
    seen: list[httpx.Request] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request)
        body = json.loads(request.content)
        assert request.method == "POST"
        assert request.url.path == "/api/api-keys/"
        assert DASHBOARD_SESSION_COOKIE in request.headers["cookie"]
        assert body["name"] == "gha:bioden:codex-pr-review.yml:26446723001:1"
        assert body["expiresAt"].endswith("Z")
        assert body["limits"] == [
            {
                "limitType": "cost_usd",
                "limitWindow": "weekly",
                "maxValue": 50_000_000,
                "modelFilter": None,
            }
        ]
        return httpx.Response(200, json={"id": "api-key-id", "key": "sk-clb-secret", "expiresAt": body["expiresAt"]})

    client = CodexLbDashboardClient(config, transport=httpx.MockTransport(handler), session_factory=StaticSessionFactory())
    issued = client.create_key(
        repository_name="bioden",
        workflow_file=".github/workflows/codex-pr-review.yml",
        run_id="26446723001",
        run_attempt="1",
    )

    assert issued.relay_token == "sk-clb-secret"
    assert issued.api_key_id == "api-key-id"
    assert issued.expires_at > datetime.now(UTC) + timedelta(minutes=59)
    assert len(seen) == 1


def test_dashboard_client_delete_treats_404_as_absent(config: BrokerConfig) -> None:
    client = CodexLbDashboardClient(
        config,
        transport=httpx.MockTransport(lambda request: httpx.Response(404, json={"detail": "not found"})),
        session_factory=StaticSessionFactory(),
    )

    assert client.delete_key("missing-key") is False


def test_dashboard_client_create_failure_is_error(config: BrokerConfig) -> None:
    client = CodexLbDashboardClient(
        config,
        transport=httpx.MockTransport(lambda request: httpx.Response(500, text="boom")),
        session_factory=StaticSessionFactory(),
    )

    with pytest.raises(CodexLbClientError, match="HTTP 500"):
        client.create_key(
            repository_name="bioden",
            workflow_file=".github/workflows/codex-pr-review.yml",
            run_id="26446723001",
            run_attempt="1",
        )


def test_exchange_uses_dashboard_create_api(
    tmp_path,
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
) -> None:
    config = replace(config, broker_db_path=str(tmp_path / "broker.db"))
    dashboard_client = RecordingDashboardClient()
    app = create_app(
        config,
        jwks_client=StaticJwksClient(public_jwk),
        dashboard_client=dashboard_client,
    )

    response = TestClient(app).post("/oidc/exchange", json={"token": make_token(signing_key)})

    assert response.status_code == 200
    assert response.json()["relay_token"] == "sk-clb-created"
    assert dashboard_client.created == 1


def test_create_app_does_not_start_cleanup(public_jwk: PyJWK, config: BrokerConfig) -> None:
    class NoCleanupAuditStore:
        cleanup_checked = False

        def record_exchange(self, **kwargs) -> None:
            pass

        def list_expired_key_ids(self, **kwargs) -> list[str]:
            self.cleanup_checked = True
            raise AssertionError("broker startup must not run cleanup")

    audit_store = NoCleanupAuditStore()
    app = create_app(
        config,
        jwks_client=StaticJwksClient(public_jwk),
        audit_store=audit_store,
        dashboard_client=RecordingDashboardClient(),
    )

    with TestClient(app) as client:
        response = client.get("/oidc/health")

    assert response.status_code == 200
    assert not audit_store.cleanup_checked


def test_exchange_create_failure_does_not_record_successful_audit(
    tmp_path,
    signing_key: rsa.RSAPrivateKey,
    public_jwk: PyJWK,
    config: BrokerConfig,
) -> None:
    db_path = tmp_path / "broker.db"
    config = replace(config, broker_db_path=str(db_path))
    app = create_app(
        config,
        jwks_client=StaticJwksClient(public_jwk),
        dashboard_client=RecordingDashboardClient(fail=True),
    )

    response = TestClient(app).post("/oidc/exchange", json={"token": make_token(signing_key)})

    assert response.status_code == 500
    con = sqlite3.connect(str(db_path))
    tables = [row[0] for row in con.execute("select name from sqlite_master where type='table'")]
    assert "exchange_audit" not in tables


def test_cleanup_deletes_only_expired_audited_keys(tmp_path, config: BrokerConfig) -> None:
    store = AuditStore(str(tmp_path / "broker.db"))
    store.record_exchange(
        claims=broker_claims(),
        issued=IssuedToken("sk-clb-expired", datetime.now(UTC) - timedelta(minutes=1), "expired-key"),
    )
    store.record_exchange(
        claims=broker_claims(),
        issued=IssuedToken("sk-clb-active", datetime.now(UTC) + timedelta(minutes=30), "active-key"),
    )
    dashboard_client = RecordingDashboardClient()

    result = cleanup_cli.cleanup_expired_keys(config=config, audit_store=store, dashboard_client=dashboard_client)

    assert result.candidates == 1
    assert result.deleted == 1
    assert result.failed == 0
    assert dashboard_client.deleted == ["expired-key"]
    assert store.list_expired_key_ids(now=datetime.now(UTC)) == []
    con = sqlite3.connect(str(tmp_path / "broker.db"))
    assert con.execute("select api_key_id from exchange_audit").fetchall() == [("active-key",)]


def test_cleanup_treats_missing_key_as_completed(tmp_path, config: BrokerConfig) -> None:
    store = AuditStore(str(tmp_path / "broker.db"))
    store.record_exchange(
        claims=broker_claims(),
        issued=IssuedToken("sk-clb-missing", datetime.now(UTC) - timedelta(minutes=1), "already-gone"),
    )
    dashboard_client = RecordingDashboardClient()

    result = cleanup_cli.cleanup_expired_keys(config=config, audit_store=store, dashboard_client=dashboard_client)

    assert result.deleted == 0
    assert result.already_absent == 1
    assert store.list_expired_key_ids(now=datetime.now(UTC)) == []
    con = sqlite3.connect(str(tmp_path / "broker.db"))
    assert con.execute("select count(*) from exchange_audit").fetchone()[0] == 0


def test_cleanup_delete_failure_leaves_audit_and_returns_nonzero(tmp_path, config: BrokerConfig) -> None:
    store = AuditStore(str(tmp_path / "broker.db"))
    store.record_exchange(
        claims=broker_claims(),
        issued=IssuedToken("sk-clb-expired", datetime.now(UTC) - timedelta(minutes=1), "expired-key"),
    )
    dashboard_client = RecordingDashboardClient(delete_fail_ids={"expired-key"})

    exit_code = cleanup_cli.main([], config=config, audit_store=store, dashboard_client=dashboard_client)

    assert exit_code == 1
    assert dashboard_client.deleted == ["expired-key"]
    assert store.list_expired_key_ids(now=datetime.now(UTC)) == ["expired-key"]
