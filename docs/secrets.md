# Secrets Inventory

Do not commit these values. Store them only on the host or in Forgejo/Cloudflare
secret stores.

## Local Files

- `stacks/forgejo/.env`
  - `POSTGRES_PASSWORD`
  - `FORGEJO_TOKEN`
  - `FORGEJO_RUNNER_SECRET`
- `stacks/agent-stack/secrets/cloudflared.env`
  - `TUNNEL_TOKEN`
- `stacks/codex-github-runners/state/github_pat`
  - GitHub PAT used for runner registration

## Forgejo Secrets

- `FORGEJO_BOT_TOKEN`
- `CLOUDFLARE_API_TOKEN` for deploy-capable repositories
- Optional `CODEX_REVIEW_COMMAND` when Codex review automation is enabled

## Excluded Runtime Secrets

- Forgejo JWT and Actions private keys under the live data volume
- Forgejo SSH host private key under the live data volume
- Codex `auth.json`, sqlite state, logs, sessions, attachments, generated images
- SSH private keys under `~/.ssh`
- GitHub CLI `hosts.yml`

