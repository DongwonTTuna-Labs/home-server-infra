# Secrets Inventory

Do not commit these values. Store them only on the host or in the relevant
external secret store.

## Local Files

- `stacks/agent-stack/secrets/cloudflared.env`
  - `TUNNEL_TOKEN` for the dedicated `ssh.dongwontuna.net` tunnel
- `${HOME}/.cloudflared/codex-lb.json`
  - Legacy Cloudflare tunnel credentials for the retired codex-lb tunnel runner
- `${HOME}/.cloudflared/opencode.json`
  - Cloudflare tunnel credentials for the non-SSH `tunnel-apps` domain
- `${HOME}/.config/opencode/opencode.env`
  - OpenCode native server/update environment, including `OPENCODE_SERVER_PASSWORD`
- `${HOME}/.opencode/`
  - OpenCode native runtime, auth-adjacent state, and installed binary cache
- `${HOME}/.config/mcp-suite/`
  - User systemd update script for rebuilding the local MCP suite image
- `stacks/codex-lb/.env`
  - `CODEX_LB_POSTGRES_PASSWORD` for the codex-lb Postgres service
- `${HOME}/.config/environment.d/20-codex-lb.conf`
  - `CODEX_LB_HOME_API_KEY` for the home-server Codex localhost provider
  - Imported into the user systemd manager; restart existing Codex processes
    after rotating or restoring it
- `${HOME}/.bashrc`, `${HOME}/.bash_profile`, `${HOME}/.profile`,
  `${HOME}/.zshrc`, and `${HOME}/.zprofile`
  - Export the same `CODEX_LB_HOME_API_KEY` for interactive, login, and SSH
    shells
  - Keep every shell copy synchronized with `20-codex-lb.conf` during rotation;
    never print the value while checking consistency
- `${HOME}/.codex/ai-relay.env` on the remote Mac
  - `CODEX_LB_LOCAL_API_KEY` for the direct
    `relay-ai.dongwontuna.net` Codex provider
  - Loaded into the GUI session by the dedicated environment LaunchAgent; it is
    not an SSH tunnel configuration
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
- SSH private keys under `~/.ssh`
- GitHub CLI `hosts.yml`
