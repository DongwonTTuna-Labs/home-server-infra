# codex-lb Stack

This stack owns the Codex relay application and database. Public routing for
`relay-ai.dongwontuna.net` is owned by `stacks/tunnel-apps`.

The application image is an operator-verified stable release pinned by both tag
and OCI index digest. Both the application and Postgres are excluded from
Watchtower. Change the image only after the backup and migration preflight
below succeeds; do not switch this stack to a mutable `latest` tag.

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

The target Alembic head for the pinned stable release is
`20260713_040000_add_account_refresh_claims`. The prior beta.3 head is
`20260711_030000_add_limit_warmup_idle_threshold`. A historical 1.19 rollback
used `20260513_000000_add_accounts_alias` for both a true 1.19 schema and a
1.20.1 superset schema. The revision string alone cannot distinguish them.

Start PostgreSQL, pull the pinned images, and define read-only schema checkers:

```bash
set -euo pipefail
COMPOSE=stacks/codex-lb/compose.yaml
TARGET_HEAD=20260713_040000_add_account_refresh_claims
BETA3_HEAD=20260711_030000_add_limit_warmup_idle_threshold
BETA2_HEAD=20260709_000000_add_ttft_phase_observability
STABLE_HEAD=20260611_000000_merge_dashboard_guest_and_weekly_useragent_heads
PRE_BETA_HEAD=20260701_000000_add_weekly_pace_smoothing_minutes
V119_IMAGE='ghcr.io/soju06/codex-lb:1.19.0@sha256:732cbb2d29b3f02ddacaf5aad6458e60fb926e58a5376cab1a288b9c866ea219'
V1201_IMAGE='ghcr.io/soju06/codex-lb:1.20.1@sha256:e4ccfb16d4aa5f715e225db62862f8773667a492d486e9503e5491d2caff2052'

docker compose -f "$COMPOSE" up -d postgres
docker compose -f "$COMPOSE" pull codex-lb

db() {
  docker compose -f "$COMPOSE" run --rm --no-deps -T \
    --entrypoint python codex-lb -m app.db.migrate "$@"
}
schema_check_as() {
  local image="$1"
  docker compose -f "$COMPOSE" \
    -f <(printf 'services:\n  codex-lb:\n    image: %s\n' "$image") \
    run --rm --no-deps -T --entrypoint python codex-lb \
    -m app.db.migrate check
}

db current
```

Use this fail-closed state matrix:

| Current state | Required action |
| --- | --- |
| `TARGET_HEAD` | Run `db check`. Do not stamp backward or re-run migration manually. |
| `none` and the `public` schema has zero tables | Run `db upgrade head`, then `db current` and `db check`. |
| `BETA3_HEAD`, `BETA2_HEAD`, `STABLE_HEAD`, or `PRE_BETA_HEAD` | These are known ancestors. Run `db upgrade head` without stamping, then require `TARGET_HEAD` from `db current` and run `db check`. |
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
docker network inspect paca_mcp_internal >/dev/null 2>&1 || docker network create paca_mcp_internal
docker compose -f stacks/codex-lb/compose.yaml up -d
```

## Verify

```sh
curl -fsS http://127.0.0.1:2455/health/ready
curl -fsS https://relay-ai.dongwontuna.net/health/ready
curl -fsS -D - -o /dev/null http://127.0.0.1:2455/health/ready \
  | grep -i '^x-app-version: 1.21.0'
```

Health alone is not release evidence. Finish with one real Codex response and
confirm the matching request log reports a successful WebSocket upstream rather
than an HTTP fallback.
