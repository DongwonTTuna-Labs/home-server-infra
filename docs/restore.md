# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`.
2. Restore `${HOME}/.cloudflared/685aeec4-5771-459a-8909-7ccfbb086815.json`;
   `stacks/tunnel-apps` mounts it read-only as the credential for the
   relay/Paca tunnel. If that tunnel was revoked or deleted, create a new
   named tunnel and update the tunnel ID and credential mount together. Do not
   restore OpenCode DNS or ingress.
3. Restore Docker volumes `codex-lb-data` and
   `codex-lb_codex-lb-postgres-data`.
4. Create the independently managed private network shared with Paca and
   mcp-suite:

   ```sh
   docker network inspect paca_mcp_internal >/dev/null 2>&1 || docker network create paca_mcp_internal
   ```

5. Start PostgreSQL only. Do not start `codex-lb-stack.service` or the full
   Compose stack yet because application startup applies migrations:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d postgres
   ```

6. Create a new backup of the restored state, classify the Alembic revision and
   physical schema, and complete the fail-closed migration preflight in
   `stacks/codex-lb/README.md`.
7. Only after `current` reports the pinned image's target head and `check`
   reports `migration_policy=ok` plus `schema_drift=none`, start the application
   and validate/recreate the tunnel connector:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d codex-lb
   cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate
   docker compose -f stacks/tunnel-apps/compose.yaml config --quiet
   docker compose -f stacks/tunnel-apps/compose.yaml up -d --force-recreate cloudflared-apps
   ```

8. Restore `CODEX_LB_HOME_API_KEY` in both the user-systemd environment and all
   login/SSH shell startup surfaces listed in `docs/secrets.md`. Import or
   restart the user manager as needed, then restart every existing Codex client
   process so it inherits the restored value.
9. Before changing DNS, verify the new connector has active connections:

   ```sh
   cloudflared tunnel info tunnel-apps
   docker logs cloudflared-apps 2>&1 | grep 'Registered tunnel connection'
   ```

10. Route the retained hostnames and verify the local and public surfaces:

   ```sh
   cloudflared tunnel route dns --overwrite-dns tunnel-apps relay-ai.dongwontuna.net
   cloudflared tunnel route dns --overwrite-dns tunnel-apps paca.dongwontuna.net
   curl -fsS http://127.0.0.1:2455/health
   curl -fsS https://relay-ai.dongwontuna.net/health
   curl -fsS -o /dev/null https://relay-ai.dongwontuna.net/dashboard
   curl -fsS https://paca.dongwontuna.net/api/healthz
   ```

11. Finish with a real Codex response and confirm its matching relay request log
   reports a successful WebSocket upstream.

The retired `${HOME}/.cloudflared/codex-lb.json` credential is not required for
restore unless you are intentionally rolling back the old per-stack tunnel
runner.

## Paca

This restores the repo-managed Paca stack while preserving the `paca` Compose
project and its existing Docker volumes. Don't use volume deletion commands.

### Local Files And Volumes

1. Restore or create `stacks/paca/.env` from `stacks/paca/.env.example` on the
   host. `.env` is local-only and ignored.
2. Confirm `stacks/paca/backups/` or the configured `BACKUP_DIR` has a current
   non-empty `paca-*.sql.gz` dump before any restore or risky update.
3. Preserve these Docker volumes unless the user explicitly approves a
   destructive data replacement after backup evidence exists: `paca_postgres_data`,
   `paca_valkey_data`, `paca_minio_data`, `paca_backend_plugins`,
   `paca_frontend_plugins`, `paca_mcp_plugins`, `paca_caddy_data`, and
   `paca_caddy_config`.

### Safe Stop And Start

```sh
docker network inspect paca_mcp_internal >/dev/null 2>&1 || docker network create paca_mcp_internal
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml stop
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml up -d
```

The network is independently managed. Create it before starting any of the
three attached stacks; Paca teardown must not remove it. Restart `mcp-suite`
after the network exists:

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml up -d
docker compose -f stacks/mcp-suite/compose.yaml up -d
```

### Backup

The `db-backup` service writes gzip dumps named `paca-*.sql.gz` into
`BACKUP_DIR`, defaulting to `stacks/paca/backups/` when the stack is run from
this repo.

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T db-backup /usr/local/bin/run-backup.sh
ls -lh stacks/paca/backups/paca-*.sql.gz
```

Record only path, size, and timestamp in `.omo/evidence/`. Don't copy dump
contents into evidence.

### Restore Postgres Data

Restore only after explicit approval and backup evidence. Stop app writers,
keep Postgres running, then load the selected clean dump through stdin. The
backup contains `DROP ... IF EXISTS` statements, and the restore aborts and
rolls back on the first SQL error:

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml stop api realtime ai-agent gateway web db-backup
gunzip -c stacks/paca/backups/paca-YYYYMMDD-HHMMSS.sql.gz | docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres sh -c 'psql -v ON_ERROR_STOP=1 --single-transaction -U "$POSTGRES_USER" -d "$POSTGRES_DB"'
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml up -d
```

Use the real backup filename from evidence. Don't paste DB dump contents into
the terminal log or task evidence.

### ENCRYPTION_KEY Rotation

`ENCRYPTION_KEY` is not a normal replace-and-restart secret. It encrypts
`agents.llm_api_key_secret` values in Postgres and is read by both `api` and
`ai-agent`.

Restore the exact previous value. Rotation is blocked unless a reviewed
transaction decrypts every affected row with the old key, re-encrypts it with
the new 64 hex char key, and proves the before/after row counts. If any decrypt
or count check fails, roll back the transaction and stop the restore.

### Relay Policy And MCP Seed Procedure

Paca reaches local MCP servers over the private Docker network
`paca_mcp_internal`. After API migrations or a database restore, apply the
relay-only agent policy and then seed per-agent MCP rows:

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres sh -c 'psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"' < stacks/paca/relay-ai-enforce.sql
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres sh -c 'psql -v ON_ERROR_STOP=1 -U "$POSTGRES_USER" -d "$POSTGRES_DB"' < stacks/paca/mcp-local-servers.sql
```

The relay script installs the trigger/check constraint and updates active agent
rows to `ai-relay`, `gpt-5.5`, and the private codex-lb URL. Both commands stop
on the first SQL error.

The seed should add or update `lsp`, `codegraph`, and `agbrowse` rows with
`http://mcp-suite:8301/mcp`, `http://mcp-suite:8302/mcp`, and
`http://mcp-suite:8303/mcp`. These URLs stay private and must not be added to
Cloudflare Tunnel.

### Cutover Smoke

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml ps
curl -fsS http://127.0.0.1:3080/api/healthz
curl -fsS https://paca.dongwontuna.net/api/healthz
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres sh -c 'pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"'
docker ps --filter 'label=com.centurylinklabs.watchtower.enable=true' --format '{{.Names}}'
```

Expected Watchtower-enabled Paca services are only `ai-agent` and `postgres`.
Confirm `api`, `web`, `realtime`, `gateway`, `minio`, `valkey`, and `db-backup`
are absent or false.

### Rollback

Rollback starts from recorded evidence, not guesses:

1. Keep the current backup path, prior image refs, and smoke results in
   `.omo/evidence/`.
2. Pin the prior image tag or digest in `stacks/paca/.env` or Compose.
3. Start the stack again with the safe start command above.
4. Re-run local health, public health, DB health, xhigh drift check, MCP smoke,
   and Watchtower label checks.
5. If Postgres data must be restored, use the restore section only after the
   user approves the exact backup file.

Watchtower hooks can detect and log a bad update, but they can't block it from
being applied. Rollback evidence is required for `ai-agent` and Postgres.
