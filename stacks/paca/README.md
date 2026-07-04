# Paca Stack

Repo-managed Paca runs as one Compose project named `paca`. Keep this as one
stack folder. Don't create per-service folders for `api`, `web`, `realtime`,
`gateway`, `minio`, `valkey`, `postgres`, `db-backup`, or `ai-agent` unless the
official deploy shape changes.

## Layout

```text
stacks/paca/
  compose.yaml
  docker-compose.override.yaml
  .env.example
  caddy/Caddyfile
  relay-ai-enforce.sql
  mcp-local-servers.sql
```

`stacks/paca/.env.example` documents required names. `stacks/paca/.env` holds
real values, is local-only, and is ignored.

## Data And Backups

The Compose project name stays `paca` so existing volumes are reused:

- `paca_postgres_data`
- `paca_valkey_data`
- `paca_minio_data`
- `paca_backend_plugins`
- `paca_frontend_plugins`
- `paca_mcp_plugins`
- `paca_caddy_data`
- `paca_caddy_config`

`db-backup` writes `paca-*.sql.gz` files to `BACKUP_DIR`, expected locally as
`stacks/paca/backups/` unless `.env` points elsewhere. Backups are ignored.

Run a fresh backup before cutover, risky updates, or restore:

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T db-backup /usr/local/bin/run-backup.sh
ls -lh stacks/paca/backups/paca-*.sql.gz
```

## Run

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml up -d
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml stop
```

Don't delete volumes during normal stop, restart, rollback, or restore.

## MCP Seed

Paca uses the private Docker network `paca_mcp_internal`. `mcp-suite` joins that
network as `mcp-suite`, and Paca agents use per-agent MCP rows seeded by
`mcp-local-servers.sql`.

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" < stacks/paca/mcp-local-servers.sql
```

The seed registers `lsp`, `codegraph`, and `agbrowse` with these private URLs:

- `http://mcp-suite:8301/mcp`
- `http://mcp-suite:8302/mcp`
- `http://mcp-suite:8303/mcp`

These URLs stay off Cloudflare Tunnel.

## ENCRYPTION_KEY

`ENCRYPTION_KEY` protects encrypted agent LLM keys in Postgres. It must be 64
hex chars and must match between `api` and `ai-agent`.

Changing it blindly breaks decrypt in `ai-agent`. Rotate it only by decrypting
existing encrypted agent LLM keys, re-encrypting them with the new key in one
transaction, and verifying the row count. If that can't be proven, abort before
changing the key and manually re-enter the agent LLM keys after cutover.

## Watchtower Labels

Paca `ai-agent` and `postgres` are explicit user-approved high-risk auto-update
exceptions. Every other Paca service stays false or absent.

| Service | Watchtower label | Reason |
| --- | --- | --- |
| `ai-agent` | `true` | User-approved exception, requires xhigh drift detection and rollback evidence |
| `postgres` | `true` | User-approved exception, requires backup, DB health check, and rollback evidence |
| `api` | `false` or absent | Not approved for auto-update |
| `web` | `false` or absent | Not approved for auto-update |
| `realtime` | `false` or absent | Not approved for auto-update |
| `gateway` | `false` or absent | Not approved for auto-update |
| `minio` | `false` or absent | Not approved for auto-update |
| `valkey` | `false` or absent | Not approved for auto-update |
| `db-backup` | `false` or absent | Not approved for auto-update |

Watchtower hooks can't block bad updates. They only detect and log. Keep backup,
smoke, and rollback evidence under `.omo/evidence/`, which is local-only and
ignored.

## Smoke

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml ps
curl -fsS http://127.0.0.1:3080/api/healthz
curl -fsS https://paca.dongwontuna.net/api/healthz
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"
```
