from __future__ import annotations

import json
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Protocol

import httpx
from cryptography.fernet import Fernet

from app.models import IssuedToken

DASHBOARD_SESSION_COOKIE = "codex_lb_dashboard_session"


class CodexLbClientError(RuntimeError):
    pass


class DashboardClientConfig(Protocol):
    token_ttl_seconds: int
    codex_lb_base_url: str
    codex_lb_encryption_key_path: str
    dashboard_session_ttl_seconds: int
    http_timeout_seconds: float
    api_key_cost_limit_microdollars: int
    api_key_cost_limit_window: str


def _utc_now() -> datetime:
    return datetime.now(UTC)


def _api_datetime(value: datetime) -> str:
    return value.astimezone(UTC).isoformat().replace("+00:00", "Z")


def _parse_api_datetime(value: object) -> datetime:
    if not isinstance(value, str) or not value:
        raise CodexLbClientError("codex-lb response is missing expiresAt")
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    if parsed.tzinfo is None:
        return parsed.replace(tzinfo=UTC)
    return parsed.astimezone(UTC)


class DashboardSessionCookieFactory:
    def __init__(self, *, encryption_key_path: str, ttl_seconds: int) -> None:
        self.encryption_key_path = encryption_key_path
        self.ttl_seconds = ttl_seconds

    def create(self) -> str:
        key = Path(self.encryption_key_path).read_bytes()
        payload = json.dumps(
            {
                "exp": int(_utc_now().timestamp()) + self.ttl_seconds,
                "pw": True,
                "tv": True,
            },
            separators=(",", ":"),
        )
        return Fernet(key).encrypt(payload.encode("utf-8")).decode("ascii")


class CodexLbDashboardClient:
    def __init__(
        self,
        config: DashboardClientConfig,
        *,
        transport: httpx.BaseTransport | None = None,
        session_factory: DashboardSessionCookieFactory | None = None,
    ) -> None:
        self.config = config
        self._transport = transport
        self._session_factory = session_factory or DashboardSessionCookieFactory(
            encryption_key_path=config.codex_lb_encryption_key_path,
            ttl_seconds=config.dashboard_session_ttl_seconds,
        )

    def create_key(
        self,
        *,
        repository_name: str,
        workflow_file: str,
        run_id: str,
        run_attempt: str,
    ) -> IssuedToken:
        expires_at = _utc_now() + timedelta(seconds=self.config.token_ttl_seconds)
        workflow_name = Path(workflow_file).name
        response = self._request(
            "POST",
            "/api/api-keys/",
            json={
                "name": f"gha:{repository_name}:{workflow_name}:{run_id}:{run_attempt}",
                "expiresAt": _api_datetime(expires_at),
                "limits": [
                    {
                        "limitType": "cost_usd",
                        "limitWindow": self.config.api_key_cost_limit_window,
                        "maxValue": self.config.api_key_cost_limit_microdollars,
                        "modelFilter": None,
                    }
                ],
            },
        )
        if response.status_code != 200:
            raise CodexLbClientError(_response_error("create API key", response))

        data = response.json()
        relay_token = data.get("key")
        api_key_id = data.get("id")
        if not isinstance(relay_token, str) or not relay_token:
            raise CodexLbClientError("codex-lb response is missing key")
        if not isinstance(api_key_id, str) or not api_key_id:
            raise CodexLbClientError("codex-lb response is missing id")
        return IssuedToken(
            relay_token=relay_token,
            expires_at=_parse_api_datetime(data.get("expiresAt")),
            api_key_id=api_key_id,
        )

    def delete_key(self, api_key_id: str) -> bool:
        response = self._request("DELETE", f"/api/api-keys/{api_key_id}")
        if response.status_code == 204:
            return True
        if response.status_code == 404:
            return False
        raise CodexLbClientError(_response_error(f"delete API key {api_key_id}", response))

    def _request(self, method: str, path: str, **kwargs: object) -> httpx.Response:
        cookies = {DASHBOARD_SESSION_COOKIE: self._session_factory.create()}
        try:
            with httpx.Client(
                base_url=self.config.codex_lb_base_url,
                timeout=self.config.http_timeout_seconds,
                transport=self._transport,
                cookies=cookies,
            ) as client:
                return client.request(method, path, **kwargs)
        except httpx.HTTPError as exc:
            raise CodexLbClientError(f"codex-lb dashboard API request failed: {exc}") from exc


def _response_error(action: str, response: httpx.Response) -> str:
    body = response.text.strip()
    if len(body) > 500:
        body = body[:500] + "..."
    return f"Unable to {action}: HTTP {response.status_code}: {body}"
