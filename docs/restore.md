# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore or recreate `${HOME}/.cloudflared/codex-lb.json`.
2. Restore Docker volume `codex-lb-data`.
3. Start the stack:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d
   ```

4. Verify:

   ```sh
   curl -fsS https://relay-ai.dongwontuna.net/health/ready
   ```
