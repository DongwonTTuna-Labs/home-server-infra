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

## SSH Client

Install `dotfiles/ssh/config.d/forgejo-cloudflared.conf` into `~/.ssh/config`
or include it from there. The referenced SSH private key is not stored here.

