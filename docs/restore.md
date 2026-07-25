# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`.
2. Restore `${HOME}/.cloudflared/685aeec4-5771-459a-8909-7ccfbb086815.json`;
   `stacks/tunnel-apps` mounts it read-only as the credential for the
   relay/NVIDIA tunnel. If that tunnel was revoked or deleted, create a new
   named tunnel and update the tunnel ID and credential mount together. Do not
   restore OpenCode DNS or ingress.
3. Restore Docker volumes `codex-lb-data` and
   `codex-lb_codex-lb-postgres-data`.
4. Start PostgreSQL only. Do not start `codex-lb-stack.service` or the full
   Compose stack yet because application startup applies migrations:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d postgres
   ```

5. Create a new backup of the restored state, classify the Alembic revision and
   physical schema, and complete the fail-closed migration preflight in
   `stacks/codex-lb/README.md`.
6. Only after `current` reports the pinned image's target head and `check`
   reports `migration_policy=ok` plus `schema_drift=none`, start the application
   and validate/recreate the tunnel connector:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d codex-lb
   cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate
   docker compose -f stacks/tunnel-apps/compose.yaml config --quiet
   docker compose -f stacks/tunnel-apps/compose.yaml up -d --force-recreate cloudflared-apps
   ```

7. Restore `CODEX_LB_HOME_API_KEY` in both the user-systemd environment and all
   login/SSH shell startup surfaces listed in `docs/secrets.md`. Import or
   restart the user manager as needed, then restart every existing Codex client
   process so it inherits the restored value.
8. Before changing DNS, verify the new connector has active connections:

   ```sh
   cloudflared tunnel info tunnel-apps
   docker logs cloudflared-apps 2>&1 | grep 'Registered tunnel connection'
   ```

9. Route the retained hostnames and verify the local and public surfaces:

   ```sh
   cloudflared tunnel route dns --overwrite-dns tunnel-apps relay-ai.dongwontuna.net
   cloudflared tunnel route dns --overwrite-dns tunnel-apps nvidia-lb.dongwontuna.net
   curl -fsS http://127.0.0.1:2455/health
   curl -fsS https://relay-ai.dongwontuna.net/health
   curl -fsS -o /dev/null https://relay-ai.dongwontuna.net/dashboard
   curl -fsS https://nvidia-lb.dongwontuna.net/health/live
   ```

10. Finish with a real Codex response and confirm its matching relay request log
    reports a successful WebSocket upstream.

The retired `${HOME}/.cloudflared/codex-lb.json` credential is not required for
restore unless you are intentionally rolling back the old per-stack tunnel
runner.
