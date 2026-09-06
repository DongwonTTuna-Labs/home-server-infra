# codex-lb Stack

This stack owns the Codex relay application and database. Public routing for
`relay-ai.dongwontuna.net` is owned by `stacks/tunnel-apps`.

The application runs upstream **main** at
[`5ad638b6a4c9`](https://github.com/Soju06/codex-lb/commit/5ad638b6a4c9c094bcc8866b1d7487173fe3b54e)
(2026-09-05), built with the upstream Dockerfile and no source patches. There is
no published `main` image, so Compose records the full Git commit as its build
context and names the local image `codex-lb-local:main-5ad638b6a4c9`. Both the
application and Postgres remain excluded from Watchtower. Build before the
backup and migration preflight; deploy only after those checks pass.

The upstream application version still reads `1.25.0-beta.1`; use the OCI
`org.opencontainers.image.revision` label to identify this newer main build.
A restart reuses the local image. Rebuild the same source with:

```sh
docker compose -f stacks/codex-lb/compose.yaml build --pull codex-lb
```

This main revision adds the packaged Rust native HTTP/WebSocket egress worker,
which is discovered automatically. Its migration graph is unchanged from
`1.25.0-beta.1`. Keep that previous image for code rollback. The bridge sweeper
can abandon expired ambiguous operations, so retain the database and data
volume backups as well if an application-state rollback becomes necessary.

`CODEX_LB_MODEL_REGISTRY_CLIENT_VERSION` is set to the installed Codex CLI
version, `0.153.4`. It supplies the outbound client version before the live
GitHub/npm lookup completes. Upstream's `0.144.0` default caused Astra to reject
some requests during the first five minutes after restart, despite readiness
and requests carrying a current native client version succeeding. The live
version lookup continues normally after its first refresh.

## Tracked

- `compose.yaml`

## Host State

These are required on each host but are not committed:

- `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`
- Docker volume `codex-lb-data`
- Docker volume `codex-lb_codex-lb-postgres-data`

`codex-lb-data` contains the relay encryption key and must be backed up together
with PostgreSQL. A database dump without that volume is not a complete backup.

## Client Routing

The home-server Codex client uses the loopback-only listener from the tracked
`dotfiles/codex/config.toml`:

- provider: `codex-lb`
- base URL: `http://127.0.0.1:2455/backend-api/codex`
- API: `responses`
- API key environment variable: `CODEX_LB_HOME_API_KEY`
- WebSocket support: enabled

A remote Mac connects directly through Cloudflare Tunnel instead of running a
local relay or a persistent SSH port forward:

```toml
[model_providers.codex-lb]
name = "openai"
base_url = "https://relay-ai.dongwontuna.net/backend-api/codex"
wire_api = "responses"
env_key = "CODEX_LB_LOCAL_API_KEY"
supports_websockets = true
requires_openai_auth = true
```

Codex derives the secure WebSocket endpoint from the HTTPS base URL. Keep the
home-server and Mac API keys separate, and never commit either value.

## Single-User Concurrency

This relay is private to one operator, so the Compose stack sets both
`CODEX_LB_PROXY_ACCOUNT_RESPONSE_CREATE_LIMIT` and
`CODEX_LB_PROXY_ACCOUNT_STREAM_LIMIT` to `0`. In codex-lb, zero disables these
local per-account caps. Global admission limits and upstream/provider rate
limits remain active, so this removes artificial `account_response_create_cap`
and `account_stream_cap` failures without bypassing provider enforcement.

## Telemetry

v1.24.0 added anonymous usage telemetry that posts to
`https://telemetry.tokmaxxing.com`. Its consent state defaults to `undecided`,
and `undecided` resolves to **active** until someone answers the dashboard
prompt, so an upgrade from v1.23.0 would start sending without an explicit
decision. The stack therefore sets `CODEX_LB_TELEMETRY_ENABLED: "false"`, which
resolves consent from the environment (`source=env`) and outranks any persisted
dashboard answer. Delete that variable to hand the decision back to the
dashboard; set it to `"true"` to opt in deliberately.

## PostgreSQL Shared Memory

The `postgres` service sets `shm_size: 1gb`. Docker's 64MB default for
`/dev/shm` makes parallel hash joins abort with `could not resize shared memory
segment`, which asyncpg surfaces as `DiskFullError` on the request path
(upstream PR #1791, which set the same value in the upstream Compose file).
`shm_size` is a tmpfs ceiling, not a reservation, so it costs nothing until
PostgreSQL actually uses it.

## Backup Before Migration

Run this from the repository root while PostgreSQL is healthy:

```bash
set -euo pipefail
umask 077
backup_dir="${HOME}/backups/codex-lb/pre-upgrade-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$backup_dir"

docker exec codex-lb-postgres \
  pg_dump -U codex_lb -d codex_lb -Fc >"$backup_dir/postgres.dump"
docker run --rm --pull=missing \
  -v codex-lb-data:/source:ro \
  -v "$backup_dir:/backup" \
  alpine:3.22 tar -C /source -czf /backup/codex-lb-data.tgz .
docker run --rm \
  -v "$backup_dir:/backup" \
  alpine:3.22 chown "$(id -u):$(id -g)" /backup/codex-lb-data.tgz
chmod 600 "$backup_dir/codex-lb-data.tgz"

test -s "$backup_dir/postgres.dump"
test -s "$backup_dir/codex-lb-data.tgz"
docker exec -i codex-lb-postgres pg_restore -l \
  <"$backup_dir/postgres.dump" >/dev/null
gzip -t "$backup_dir/codex-lb-data.tgz"
sha256sum "$backup_dir/postgres.dump" "$backup_dir/codex-lb-data.tgz" \
  >"$backup_dir/SHA256SUMS"
sha256sum -c "$backup_dir/SHA256SUMS"
```

Retain the previous application image until the new version passes the response
smoke test. Do not commit or attach the backup files to a PR.

## Migration Preflight

The target Alembic head for the pinned main commit is
`20260830_000000_add_quota_warmup_claim_expiry`. Never read a head off the
migration file names: the graph is merged and the newest filename is routinely
an ancestor rather than the head. Read it from the image instead:

```bash
docker run --rm --entrypoint python \
  codex-lb-local:main-5ad638b6a4c9 -c "
from alembic.script import ScriptDirectory
from app.db.migrate import _build_alembic_config
print(ScriptDirectory.from_config(
    _build_alembic_config('sqlite+aiosqlite:///tmp.db')).get_heads())
"
```

Known ancestor heads, newest first: v1.24.0
`20260816_000000_add_model_source_embeddings`, v1.23.0
`20260806_120000_add_http_bridge_owner_process_epoch`, v1.22.0
`20260722_000000_backfill_request_log_useragent_families`, v1.21.0
`20260713_040000_add_account_refresh_claims`, beta.3
`20260711_030000_add_limit_warmup_idle_threshold`. A historical 1.19 rollback
used `20260513_000000_add_accounts_alias` for both a true 1.19 schema and a
1.20.1 superset schema; the revision string alone cannot distinguish them.

The v1.23.0 to v1.25.0-beta.1 jump adds no bulk backfill and completed in about
five seconds on this dataset. That is unlike the v1.22 to v1.23 upgrade, whose
hourly-rollup backfill ran for minutes; do not assume every future jump is
cheap. v1.23.0 also enabled Auth Guardian (background OAuth token keepalive,
12h staleness gate) by default; `CODEX_LB_AUTH_GUARDIAN_ENABLED=false` is the
opt-out if it ever misbehaves.

Build the candidate while the application is available. Before any migration,
stop the application and define the read-only schema checkers below. This keeps
it from serving against a half-migrated schema; startup migration stays enabled
as the fallback for an unattended restart. At this main revision the current
beta schema is already at `TARGET_HEAD`, so `db check` is sufficient:

```bash
set -euo pipefail
COMPOSE=stacks/codex-lb/compose.yaml
TARGET_HEAD=20260830_000000_add_quota_warmup_claim_expiry
V124_HEAD=20260816_000000_add_model_source_embeddings
V123_HEAD=20260806_120000_add_http_bridge_owner_process_epoch
V122_HEAD=20260722_000000_backfill_request_log_useragent_families
V121_HEAD=20260713_040000_add_account_refresh_claims
BETA3_HEAD=20260711_030000_add_limit_warmup_idle_threshold
BETA2_HEAD=20260709_000000_add_ttft_phase_observability
STABLE_HEAD=20260611_000000_merge_dashboard_guest_and_weekly_useragent_heads
PRE_BETA_HEAD=20260701_000000_add_weekly_pace_smoothing_minutes
V119_IMAGE='ghcr.io/soju06/codex-lb:1.19.0@sha256:732cbb2d29b3f02ddacaf5aad6458e60fb926e58a5376cab1a288b9c866ea219'
V1201_IMAGE='ghcr.io/soju06/codex-lb:1.20.1@sha256:e4ccfb16d4aa5f715e225db62862f8773667a492d486e9503e5491d2caff2052'

docker compose -f "$COMPOSE" build --pull codex-lb
docker compose -f "$COMPOSE" stop --timeout 60 codex-lb
docker compose -f "$COMPOSE" up -d postgres

db() {
  docker compose -f "$COMPOSE" run --rm --no-deps -T \
    --entrypoint python codex-lb -m app.db.migrate "$@" < /dev/null
}
schema_check_as() {
  local image="$1"
  docker compose -f "$COMPOSE" \
    -f <(printf 'services:\n  codex-lb:\n    image: %s\n    build: !reset null\n' "$image") \
    run --rm --no-deps -T --entrypoint python codex-lb \
    -m app.db.migrate check < /dev/null
}

db current
```

Use this fail-closed state matrix:

| Current state | Required action |
| --- | --- |
| `TARGET_HEAD` | Run `db check`. Do not stamp backward or re-run migration manually. |
| `none` and the `public` schema has zero tables | Run `db upgrade head`, then `db current` and `db check`. |
| `V124_HEAD`, `V123_HEAD`, `V122_HEAD`, `V121_HEAD`, `BETA3_HEAD`, `BETA2_HEAD`, `STABLE_HEAD`, or `PRE_BETA_HEAD` | These are known ancestors. Run `db upgrade head` without stamping, then require `TARGET_HEAD` from `db current` and run `db check`. |
| `20260513...`, 1.19 check passes and 1.20.1 check fails | This is an honest 1.19 schema. Run `db upgrade head` without stamping, then `db current` and `db check`. |
| `20260513...`, 1.19 check fails and 1.20.1 check passes | This is the rollback-stamped 1.20.1 superset. Run `db stamp "$STABLE_HEAD"`, confirm with `db current`, then run `db upgrade head`, `db current`, and `db check`. |
| Both schema checks pass, both fail, or the revision is unexpected | Stop. Do not stamp or upgrade until the physical schema and backup evidence are reconciled. |

For `none`, verify the database is actually empty before upgrading:

```bash
docker exec codex-lb-postgres psql -U codex_lb -d codex_lb -Atc \
  "SELECT count(*) FROM pg_tables WHERE schemaname = 'public';"
```

For `20260513...`, record both results before choosing a branch:

```bash
if schema_check_as "$V119_IMAGE"; then V119_OK=1; else V119_OK=0; fi
if schema_check_as "$V1201_IMAGE"; then V1201_OK=1; else V1201_OK=0; fi
printf 'v1.19=%s v1.20.1=%s\n' "$V119_OK" "$V1201_OK"
```

The final `db check` must print both `migration_policy=ok` and
`schema_drift=none` before the application is started.

## Deploy

```sh
docker compose -f stacks/codex-lb/compose.yaml up -d --no-deps --no-build --pull never --timeout 60 codex-lb
```

When the control session itself uses this relay, run the replacement and its
health/response checks in a detached host process. Keep automatic code rollback
armed until readiness and a real Codex response both pass; an SSH or chat
connection loss must not interrupt recovery.

For a code rollback to `1.25.0-beta.1`, restore its image reference from the
comment in `compose.yaml`, remove the main `build` block, and run the same
application-only command. The schema is identical for this pair of versions;
do not stamp or downgrade it. Preserve unrelated edits when restoring config.

Changing `shm_size` recreates the `postgres` container. That is expected; the
data lives on the volume.

## Verify

```sh
curl -fsS http://127.0.0.1:2455/health/ready
curl -fsS -D - -o /dev/null http://127.0.0.1:2455/health/ready \
  | grep -i '^x-app-version: 1.25.0-beta.1'
docker inspect codex-lb --format '{{index .Config.Labels "org.opencontainers.image.revision"}}'
docker exec codex-lb python -c 'from app.core.config.settings import get_settings; print(get_settings().telemetry_enabled)'
```

Check the public hostname from a browser session, not with a bare `curl`.
`relay-ai.dongwontuna.net` sits behind Cloudflare Access, so an unauthenticated
request to `/health/ready` answers `302` to the Access login. That redirect is
edge policy owned by `stacks/tunnel-apps`, not an application fault.

Health alone is not release evidence. Finish with one real Codex response
through the public base URL and confirm the matching request log reports a
WebSocket upstream rather than an HTTP fallback. Since v1.25.0-beta.1 the proxy
falls back to HTTP transport when the upstream WebSocket is unavailable, so this
check now separates a healthy path from a silently degraded one:

```sh
docker exec codex-lb-postgres psql -U codex_lb -d codex_lb -c \
  "SELECT requested_at, model, transport, upstream_transport, status
     FROM request_logs ORDER BY requested_at DESC LIMIT 5;"
```

Both `transport` and `upstream_transport` must read `websocket` with
`status = success`.

For a streaming HTTP probe, collect `response.output_text.delta` events and
require a successful `response.completed` event. The Codex endpoint can leave
`response.completed.output` empty even when the streamed answer is complete;
checking only that final array produces a false failure.

## Dashboard Staleness Note

Between 2026-08-22 and this upgrade the host ran a locally rebuilt
`codex-lb-local:1.23.0-status-resume-6dc2550` image whose only difference from
upstream v1.23.0 was two React Query options on the dashboard status bar
(`refetchOnWindowFocus: 'always'`, `refetchOnReconnect: 'always'`). That fork
was never committed here, so the repository and the host had drifted. The
v1.25.0-beta.1 upgrade dropped that fork. The current main build also uses
unmodified upstream source.

The upstream behaviour is therefore back: the status bar polls every 60s with
`refetchIntervalInBackground: false` and the global default
`refetchOnWindowFocus: false`, so after a backgrounded tab regains focus it can
show up to a minute of stale status. Nothing else is affected — no request path,
no data. Do not rebuild a private image for this; if it becomes annoying, send
the two-line change upstream.
