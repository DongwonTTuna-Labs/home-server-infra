# Secrets Inventory

Do not commit these values. Store them only on the host or in the relevant
external secret store.

## Local Files

- `stacks/agent-stack/secrets/cloudflared.env`
  - `TUNNEL_TOKEN` for the dedicated `ssh.dongwontuna.net` tunnel
- `${HOME}/.cloudflared/codex-lb.json`
  - Legacy Cloudflare tunnel credentials for the retired codex-lb tunnel runner
- `${HOME}/.cloudflared/685aeec4-5771-459a-8909-7ccfbb086815.json`
  - Cloudflare tunnel credential for the relay/NVIDIA `tunnel-apps` domain
- `stacks/codex-lb/.env`
  - `CODEX_LB_POSTGRES_PASSWORD` for the codex-lb Postgres service
- `/opt/nvidia-build-lb/secrets/admin_token`
  - Local owner-administration bearer for the NVIDIA gateway. It uses the
    `nblb_admin_` prefix and is never passed through Compose environment values.
- `/opt/nvidia-build-lb/secrets/vault_master_key`
  - Raw 32-byte encryption key for stored NVIDIA credentials. Back it up only
    as the matching half of a verified database backup pair; never rotate it
    independently of existing ciphertext.
- `/opt/nvidia-build-lb/secrets/db_password`
  - PostgreSQL password used only through the stack's file-secret boundary.
- `/opt/agent-apps/data/hermes/.env`
  - `NVIDIA_API_KEY` is the one-time `nvidia-build-lb` downstream bearer used
    by Hermes after cutover, not an NVIDIA upstream credential. Its client has
    only `models:read` and `chat:write`.
- `/opt/nvidia-build-lb/hermes-cutover-backups/`
  - Host-only, root-owned mode-`0700` rollback generations created by the
    release-matched Rust Hermes cutover helper. Files are mode `0600`; a manifest
    records checksums and internal client IDs but never a bearer. This path is
    outside every Hermes bind mount. A generation containing a former direct
    NVIDIA credential remains temporary custody only until provider-side
    revocation and the final downstream reapply are confirmed. Retired
    shell/Python quarantine helpers are not rollback authorities.
- `/opt/nvidia-build-lb/hermes-cutover-state/`
  - Root-owned mode-`0700` lock and secret-free transaction journal. A
    nonterminal journal is a fail-closed recovery condition, not permission to
    overwrite either live Hermes file manually.
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
- `nvidia-build-lb_db-data` Docker volume, including encrypted NVIDIA
  credentials, routing state, downstream-token digests, and operator evidence
- SSH private keys under `~/.ssh`
- GitHub CLI `hosts.yml`
