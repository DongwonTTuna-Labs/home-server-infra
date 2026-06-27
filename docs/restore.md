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
