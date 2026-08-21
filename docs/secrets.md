# Secrets Inventory

Do not commit these values. Store them only on the host or in the relevant
external secret store.

## Local Files

- `stacks/agent-stack/secrets/cloudflared.env`
  - `TUNNEL_TOKEN` for the dedicated `ssh.dongwontuna.net` tunnel
- `${HOME}/.cloudflared/codex-lb.json`
  - Legacy Cloudflare tunnel credentials for the retired codex-lb tunnel runner
- `${HOME}/.cloudflared/685aeec4-5771-459a-8909-7ccfbb086815.json`
  - Cloudflare tunnel credential for the relay/Orca `tunnel-apps` domain
- `${HOME}/.local/state/orca-home/serve-ready.json`
  - Mode-`0600` Orca readiness state containing remote runtime pairing authorization
  - Regenerated on service start; never commit, log, screenshot, or paste it
- `${HOME}/.config/orca/` and `${HOME}/.config/Orca/`
  - Orca profiles containing device credentials, E2EE keys, cookies, and state
  - Back up and restore only as the profile half of a matching release rollback
- `${HOME}/.cloudflared/efdf4f6b-c5ee-4673-b682-eda9a0ef71ca.json`
  - Mode-`0400` Cloudflare tunnel credential for the dedicated
    `codexpro-home` Named Tunnel
- `${HOME}/.codexpro/profiles/`
  - Mode-`0700` directory containing mode-`0600` CodexPro workspace profiles
  - The profile for `/home/dongwonttuna` contains the connector bearer token;
    restore it only when the existing ChatGPT connector URL must stay valid
- `${HOME}/.codexpro/runtime/` and
  `${HOME}/.codexpro/current-server-url.txt`
  - Mode-`0700` runtime directory and mode-`0600` generated connector state
  - Treat the whole state as secret because the URL file includes the bearer;
    runtime files are regenerated when `codexpro-home.service` starts
- `stacks/codex-lb/.env`
  - `CODEX_LB_POSTGRES_PASSWORD` for the codex-lb Postgres service
- `/opt/agent-apps/data/hermes/.env`
  - `CODEX_LB_API_KEY` is the codex-lb relay bearer Hermes authenticates with.
    The key pins `gpt-5.6-terra` at `xhigh` reasoning server-side, so it carries
    no model choice of its own and is useless against any other endpoint.
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
- `.omo/evidence/`
  - Local-only task evidence. It is ignored and must not be committed.

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
