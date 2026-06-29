# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`.
2. Restore `${HOME}/.cloudflared/opencode.json`; `stacks/tunnel-apps`
   mounts it as the credentials file for the shared non-SSH tunnel.
3. Restore Docker volumes `codex-lb-data` and
   `codex-lb_codex-lb-postgres-data`.
4. Start the stack:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d
   docker compose -f stacks/tunnel-apps/compose.yaml up -d
   ```

5. Verify the local relay and the shared non-SSH tunnel route:

   ```sh
   curl -fsS http://127.0.0.1:2455/health/ready
   curl -fsS https://relay-ai.dongwontuna.net/health/ready
   ```

The retired `${HOME}/.cloudflared/codex-lb.json` and
`${HOME}/.cloudflared/bbc484d5-7aa8-4caf-9ec5-15f64c6f5610.json` credentials are
not required for restore unless you are intentionally rolling back the old
per-stack tunnel runners.

## Coding Tool Native Services

1. Restore `${HOME}/.opencode/` and `${HOME}/.config/opencode/opencode.env`.
2. Install the coding domain user units and updater script:

   ```sh
   mkdir -p ~/.config/systemd/user ~/.config/opencode
   cp stacks/coding/systemd/*.service ~/.config/systemd/user/
   cp stacks/coding/systemd/*.timer ~/.config/systemd/user/
   cp stacks/coding/systemd/*.target ~/.config/systemd/user/
   cp stacks/coding/systemd/opencode-update.sh ~/.config/opencode/
   chmod 0755 ~/.config/opencode/opencode-update.sh
   systemctl --user daemon-reload
   systemctl --user enable --now coding-tools.target codex-cli-update.timer opencode-update.timer opencode.service
   ```

3. Verify the coding domain:

   ```sh
   systemctl --user list-dependencies --plain coding-tools.target
   systemctl --user is-active opencode.service codex-cli-update.timer opencode-update.timer
   curl -fsS http://127.0.0.1:4096/session/status -u "opencode:$OPENCODE_SERVER_PASSWORD"
   ```

## codex-lb-local Relay

The optional local relay stack uses Docker Compose project-prefixed volume names
when started with the repository commands:

1. Restore `stacks/codex-lb-local/.env` with `CODEX_LB_POSTGRES_PASSWORD`.
2. Restore Docker volumes `codex-lb-local_codex-lb-local-data` and
   `codex-lb-local_codex-lb-local-postgres-data`.
3. Start the stack only if this host should run the optional local relay:

```sh
docker compose -f stacks/codex-lb-local/compose.yaml up -d
```

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
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml stop
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml up -d
```

Restart `mcp-suite` only after Paca has created the internal network:

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
keep Postgres running, then load the selected dump through stdin:

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml stop api realtime ai-agent gateway web db-backup
gunzip -c stacks/paca/backups/paca-YYYYMMDD-HHMMSS.sql.gz | docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres psql -U "$POSTGRES_USER" -d "$POSTGRES_DB"
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml up -d
```

Use the real backup filename from evidence. Don't paste DB dump contents into
the terminal log or task evidence.

### ENCRYPTION_KEY Rotation

`ENCRYPTION_KEY` is not a normal replace-and-restart secret. It encrypts
`agents.llm_api_key_secret` values in Postgres and is read by both `api` and
`ai-agent`.

Safe options:

1. Run a transaction that decrypts every existing agent LLM key with the old
   key, re-encrypts it with the new 64 hex char key, verifies the row count,
   then updates `stacks/paca/.env`.
2. Abort before changing `ENCRYPTION_KEY`, then manually re-enter each affected
   agent LLM key through Paca after cutover.

If any decrypt fails, abort. Don't keep going with a blind key change.

### MCP Seed Procedure

Paca reaches local MCP servers over the private Docker network
`paca_mcp_internal`. Seed per-agent MCP rows from `mcp-local-servers.sql` after
Paca and `mcp-suite` are on that network:

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" < stacks/paca/mcp-local-servers.sql
```

The seed should add or update `lsp`, `codegraph`, and `agbrowse` rows with
`http://mcp-suite:8301/mcp`, `http://mcp-suite:8302/mcp`, and
`http://mcp-suite:8303/mcp`. These URLs stay private and must not be added to
Cloudflare Tunnel.

### Cutover Smoke

```sh
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml -f stacks/paca/docker-compose.override.yaml ps
curl -fsS http://127.0.0.1:3080/api/healthz
curl -fsS https://paca.dongwontuna.net/api/healthz
docker compose --env-file stacks/paca/.env -f stacks/paca/compose.yaml exec -T postgres pg_isready -U "$POSTGRES_USER" -d "$POSTGRES_DB"
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
