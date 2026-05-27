from __future__ import annotations

import sqlite3
import uuid
from datetime import UTC, datetime
from pathlib import Path
from typing import Iterable

from app.models import GithubOidcClaims, IssuedToken


def _utc_now() -> datetime:
    return datetime.now(UTC)


def _sqlite_datetime(value: datetime) -> str:
    return value.astimezone(UTC).replace(tzinfo=None).strftime("%Y-%m-%d %H:%M:%S.%f")


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

    def list_expired_key_ids(self, *, now: datetime) -> list[str]:
        self._init_schema()
        with self._connect() as con:
            rows = con.execute(
                """
                select api_key_id
                from exchange_audit
                where expires_at <= ?
                group by api_key_id
                order by min(expires_at)
                """,
                (_sqlite_datetime(now),),
            ).fetchall()
        return [row[0] for row in rows]

    def delete_exchange_rows(self, api_key_ids: Iterable[str]) -> None:
        ids = sorted({api_key_id for api_key_id in api_key_ids if api_key_id})
        if not ids:
            return
        self._init_schema()
        placeholders = ", ".join("?" for _ in ids)
        with self._connect() as con:
            con.execute(f"delete from exchange_audit where api_key_id in ({placeholders})", ids)
