from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import UTC, datetime

from app.codex_lb import CodexLbClientError, CodexLbDashboardClient
from app.config import BrokerConfig
from app.store import AuditStore


@dataclass(frozen=True, slots=True)
class CleanupResult:
    candidates: int
    deleted: int
    already_absent: int
    failed: int


def _utc_now() -> datetime:
    return datetime.now(UTC)


def cleanup_expired_keys(
    *,
    config: BrokerConfig,
    audit_store: AuditStore | None = None,
    dashboard_client: CodexLbDashboardClient | None = None,
    dry_run: bool = False,
) -> CleanupResult:
    audit_store = audit_store or AuditStore(config.broker_db_path)
    dashboard_client = dashboard_client or CodexLbDashboardClient(config)
    key_ids = audit_store.list_expired_key_ids(now=_utc_now())
    if dry_run:
        return CleanupResult(candidates=len(key_ids), deleted=0, already_absent=0, failed=0)

    deleted = 0
    already_absent = 0
    failed = 0
    completed: list[str] = []
    for key_id in key_ids:
        try:
            if dashboard_client.delete_key(key_id):
                deleted += 1
            else:
                already_absent += 1
            completed.append(key_id)
        except CodexLbClientError as exc:
            failed += 1
            print(f"failed key_id={key_id}: {exc}")

    audit_store.delete_exchange_rows(completed)
    return CleanupResult(
        candidates=len(key_ids),
        deleted=deleted,
        already_absent=already_absent,
        failed=failed,
    )


def main(
    argv: list[str] | None = None,
    *,
    config: BrokerConfig | None = None,
    audit_store: AuditStore | None = None,
    dashboard_client: CodexLbDashboardClient | None = None,
) -> int:
    parser = argparse.ArgumentParser(description="Cleanup expired broker-issued codex-lb API keys")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)

    result = cleanup_expired_keys(
        config=config or BrokerConfig.from_env(),
        audit_store=audit_store,
        dashboard_client=dashboard_client,
        dry_run=bool(args.dry_run),
    )
    print(
        "expired OIDC API key cleanup: "
        f"candidates={result.candidates} "
        f"deleted={result.deleted} "
        f"already_absent={result.already_absent} "
        f"failed={result.failed} "
        f"dry_run={bool(args.dry_run)}"
    )
    return 1 if result.failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
