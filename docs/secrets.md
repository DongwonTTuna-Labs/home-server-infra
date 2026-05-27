# Secrets Inventory

Do not commit these values. Store them only on the host or in the relevant
external secret store.

## Local Files

- `stacks/agent-stack/secrets/cloudflared.env`
  - `TUNNEL_TOKEN`
- `${HOME}/.cloudflared/codex-lb.json`
  - Cloudflare tunnel credentials for `relay-ai.dongwontuna.net`
- `stacks/codex-github-runners/state/github_pat`
  - GitHub PAT used for runner registration

## Excluded Runtime Secrets

- Codex `auth.json`, sqlite state, logs, sessions, attachments, generated images
- `codex-lb-data` Docker volume, including dashboard auth state and encryption key
- `github-oidc-broker-data` Docker volume, including broker replay/audit SQLite state
- SSH private keys under `~/.ssh`
- GitHub CLI `hosts.yml`
