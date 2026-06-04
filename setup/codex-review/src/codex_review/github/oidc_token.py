"""GitHub OIDC -> codex-lb relay credential exchange (stdlib only).

The model jobs run read-only with ``id-token: write``. This module exchanges the
GitHub Actions OIDC token for a short-lived codex-lb relay key (``sk-clb-...``),
which is then handed to ``openai/codex-action`` as ``openai-api-key`` together with
``responses-api-endpoint``. No signature work happens here: the broker validates
the presented GitHub JWT, so this only needs the standard library.
"""
from __future__ import annotations

import json
import os
import urllib.error
import urllib.parse
import urllib.request

from codex_review.core.errors import ValidationError

DEFAULT_AUDIENCE = "https://relay-ai.dongwontuna.net/github-actions"
DEFAULT_BROKER_URL = "https://relay-ai.dongwontuna.net/v1/oidc/token"
DEFAULT_RESPONSES_ENDPOINT = "https://relay-ai.dongwontuna.net/v1/responses"
RELAY_KEY_PREFIX = "sk-clb-"
_USER_AGENT = "codex-review-oidc"


def _request_json(request: urllib.request.Request) -> dict:
    try:
        with urllib.request.urlopen(request, timeout=60) as response:  # noqa: S310 - fixed https hosts
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")[:800]
        raise ValidationError(f"{request.full_url} failed with HTTP {exc.code}: {body}") from exc
    except urllib.error.URLError as exc:
        raise ValidationError(f"{request.full_url} request failed: {exc.reason}") from exc


def _require_env(name: str) -> str:
    value = os.environ.get(name)
    if not value:
        raise ValidationError(f"missing required environment variable: {name}")
    return value


def request_github_oidc_token(audience: str) -> str:
    """Fetch the GitHub Actions OIDC JWT for ``audience``."""
    parts = urllib.parse.urlsplit(_require_env("ACTIONS_ID_TOKEN_REQUEST_URL"))
    query = urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
    query.append(("audience", audience))
    request_url = urllib.parse.urlunsplit(parts._replace(query=urllib.parse.urlencode(query)))
    payload = _request_json(
        urllib.request.Request(
            request_url,
            headers={
                "Accept": "application/json",
                "Authorization": f"Bearer {_require_env('ACTIONS_ID_TOKEN_REQUEST_TOKEN')}",
                "User-Agent": _USER_AGENT,
            },
        )
    )
    token = payload.get("value")
    if not isinstance(token, str) or not token:
        raise ValidationError("GitHub OIDC response did not include a token value")
    return token


def exchange_for_relay_token(oidc_token: str, broker_url: str) -> dict[str, str]:
    """Exchange a GitHub OIDC JWT for a codex-lb relay credential."""
    exchanged = _request_json(
        urllib.request.Request(
            broker_url,
            data=json.dumps({"token": oidc_token}).encode("utf-8"),
            method="POST",
            headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "User-Agent": _USER_AGENT,
            },
        )
    )
    token_type = exchanged.get("tokenType") or exchanged.get("token_type")
    relay_token = exchanged.get("apiKey") or exchanged.get("relay_token")
    expires_at = exchanged.get("expiresAt") or exchanged.get("expires_at")
    if token_type != "Bearer" or not isinstance(relay_token, str) or not relay_token.startswith(RELAY_KEY_PREFIX):
        raise ValidationError("exchange response did not include a valid relay credential")
    if not isinstance(expires_at, str) or not expires_at:
        raise ValidationError("exchange response did not include expires_at")
    return {"relay_token": relay_token, "expires_at": expires_at}


def mint_relay_token(audience: str = DEFAULT_AUDIENCE, broker_url: str = DEFAULT_BROKER_URL) -> dict[str, str]:
    """Run the full OIDC -> relay-key exchange and return the credential fields."""
    oidc_token = request_github_oidc_token(audience)
    return exchange_for_relay_token(oidc_token, broker_url)
