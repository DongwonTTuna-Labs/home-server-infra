from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime


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
