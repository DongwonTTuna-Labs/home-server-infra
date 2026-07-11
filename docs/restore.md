# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`.
2. Restore `${HOME}/.cloudflared/opencode.json`; `stacks/tunnel-apps`
   mounts it as the credentials file for the shared non-SSH tunnel.
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
   and tunnel:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d codex-lb
   docker compose -f stacks/tunnel-apps/compose.yaml up -d
   ```

7. Restore `CODEX_LB_HOME_API_KEY` in both the user-systemd environment and all
   login/SSH shell startup surfaces listed in `docs/secrets.md`. Import or
   restart the user manager as needed, then restart every existing Codex client
   process so it inherits the restored value.
8. Verify the local relay and the shared non-SSH tunnel route:

   ```sh
   curl -fsS http://127.0.0.1:2455/health/ready
   curl -fsS https://relay-ai.dongwontuna.net/health/ready
   ```

9. Finish with a real Codex response and confirm its matching relay request log
   reports a successful WebSocket upstream.

The retired `${HOME}/.cloudflared/codex-lb.json` credential is not required for
restore unless you are intentionally rolling back the old per-stack tunnel
runner.

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
