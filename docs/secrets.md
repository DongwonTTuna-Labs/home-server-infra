# Secrets Inventory

Do not commit these values. Store them only on the host or in the relevant
external secret store.

## Local Files

- `stacks/agent-stack/secrets/cloudflared.env`
  - `TUNNEL_TOKEN` for the dedicated `ssh.dongwontuna.net` tunnel
- `${HOME}/.cloudflared/codex-lb.json`
  - Legacy Cloudflare tunnel credentials for the retired codex-lb tunnel runner
- `${HOME}/.cloudflared/bbc484d5-7aa8-4caf-9ec5-15f64c6f5610.json`
  - Legacy Cloudflare tunnel credentials for the retired codex-lb-local tunnel runner
- `${HOME}/.cloudflared/opencode.json`
  - Cloudflare tunnel credentials for the non-SSH `tunnel-apps` domain
- `${HOME}/.config/opencode/opencode.env`
  - OpenCode native server/update environment, including `OPENCODE_SERVER_PASSWORD`
- `${HOME}/.opencode/`
  - OpenCode native runtime, auth-adjacent state, and installed binary cache
- `${HOME}/.config/mcp-suite/`
  - User systemd update script for rebuilding the local MCP suite image
- `${HOME}/.codex/ai-relay.env`
  - `CODEX_LB_LOCAL_API_KEY` for local codex-lb relay clients
- `stacks/codex-lb/.env`
  - `CODEX_LB_POSTGRES_PASSWORD` for the codex-lb Postgres service
- `stacks/codex-lb-local/.env`
  - `CODEX_LB_POSTGRES_PASSWORD` for the optional local relay Postgres service
- `stacks/codex-github-runners/.env`
  - `CODEX_RELAY_API_KEY` for Codex relay API access
  - `CODEX_LOOP_PAT` for Codex loop push and continuation dispatch
- `stacks/codex-github-runners/state/github_pat`
  - GitHub PAT used for runner registration

## External Secret Stores

- GitHub Actions consumer secrets for Grimoire reusable workflows
  - `GRIMOIRE_PAT`
  - `AI_RELAY_API_KEY`
  - `CF_ACCESS_CLIENT_ID`
  - `CF_ACCESS_CLIENT_SECRET`

## Excluded Runtime Secrets

- Codex `auth.json`, sqlite state, logs, sessions, attachments, generated images
- `codex-lb-data` Docker volume, including dashboard auth state and encryption
  key
- `codex-lb_codex-lb-postgres-data` Docker volume, including relay database
  state
- `codex-lb-local_codex-lb-local-data` and
  `codex-lb-local_codex-lb-local-postgres-data` Docker volumes when the optional
  local relay stack is deployed
- SSH private keys under `~/.ssh`
- GitHub CLI `hosts.yml`
