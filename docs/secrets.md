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
- `stacks/opencode/.env`
  - `OPENCODE_SERVER_PASSWORD` (HTTP basic auth) and `CODEX_LB_LOCAL_API_KEY`
    (ai-relay provider key)
- `${HOME}/.cloudflared/opencode.json`
  - Cloudflare tunnel credentials for `opencode.dongwontuna.net`

## Excluded Runtime Secrets

- Codex `auth.json`, sqlite state, logs, sessions, attachments, generated images
- `codex-lb-data` Docker volume, including dashboard auth state and encryption key
- SSH private keys under `~/.ssh`
- GitHub CLI `hosts.yml`
