# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore or recreate `${HOME}/.cloudflared/codex-lb.json`.
2. Restore Docker volumes `codex-lb-data` and `github-oidc-broker-data`.
3. Start the stack:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml build codex-lb github-oidc-broker
   docker compose -f stacks/codex-lb/compose.yaml up -d
   ```

4. Install the cleanup timer:

   ```sh
   stacks/codex-lb/scripts/install-user-timer.sh
   ```

5. Verify:

   ```sh
   curl -fsS http://127.0.0.1:2465/oidc/health
   curl -fsS https://relay-ai.dongwontuna.net/oidc/health
   systemctl --user list-timers github-oidc-broker-cleanup.timer
   ```
