# Restore Notes

This repository restores configuration, not live data.

## Forgejo

1. Copy `stacks/forgejo/.env.example` to `stacks/forgejo/.env`.
2. Fill `POSTGRES_PASSWORD`, `FORGEJO_TOKEN`, and runner values.
3. Restore PostgreSQL and Forgejo data volumes from backups.
4. Start Forgejo:

   ```sh
   docker compose --env-file stacks/forgejo/.env -f stacks/forgejo/compose.yaml up -d
   ```

5. Verify:

   ```sh
   curl http://127.0.0.1:3000/api/healthz
   forgejo admin user list
   ```

## Cloudflare Tunnel

1. Copy `stacks/agent-stack/secrets/cloudflared.env.example` to
   `stacks/agent-stack/secrets/cloudflared.env`.
2. Fill the Cloudflare tunnel token.
3. Start the tunnel stack from its deployed host path.
4. Confirm the remote tunnel ingress still routes:
   - `git.dongwontuna.net` to `http://127.0.0.1:3000`
   - `ssh.dongwontuna.net` to `ssh://127.0.0.1:2222`

## codex-lb Relay

1. Restore or recreate `${HOME}/.cloudflared/codex-lb.json`.
2. Restore Docker volumes `codex-lb-data` and `github-oidc-broker-data`.
3. Start the stack:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml build github-oidc-broker
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

## SSH Client

Install `dotfiles/ssh/config.d/forgejo-cloudflared.conf` into `~/.ssh/config`
or include it from there. The referenced SSH private key is not stored here.
