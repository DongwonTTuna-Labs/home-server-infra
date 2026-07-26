# Restore Notes

This repository restores configuration, not live data.

## codex-lb Relay

1. Restore `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`.
2. Restore `${HOME}/.cloudflared/685aeec4-5771-459a-8909-7ccfbb086815.json`;
   `stacks/tunnel-apps` mounts it read-only as the credential for the
   relay/NVIDIA/Orca tunnel. If that tunnel was revoked or deleted, create a new
   named tunnel and update the tunnel ID and credential mount together. Do not
   restore OpenCode DNS or ingress.
3. Restore Docker volumes `codex-lb-data` and
   `codex-lb_codex-lb-postgres-data`.
4. Start PostgreSQL only. Do not start `codex-lb-stack.service` or the full
   Compose stack yet because application startup applies migrations:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d postgres
   ```

5. Create a new backup of the restored state, classify the Alembic revision and
   physical schema, and complete the fail-closed migration preflight in
   `stacks/codex-lb/README.md`.
6. Only after `current` reports the pinned image's target head and `check`
   reports `migration_policy=ok` plus `schema_drift=none`, start the application
   and validate/recreate the tunnel connector:

   ```sh
   docker compose -f stacks/codex-lb/compose.yaml up -d codex-lb
   cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate
   docker compose -f stacks/tunnel-apps/compose.yaml config --quiet
   docker compose -f stacks/tunnel-apps/compose.yaml up -d --force-recreate cloudflared-apps
   ```

7. Restore `CODEX_LB_HOME_API_KEY` in both the user-systemd environment and all
   login/SSH shell startup surfaces listed in `docs/secrets.md`. Import or
   restart the user manager as needed, then restart every existing Codex client
   process so it inherits the restored value.
8. Before changing DNS, verify the new connector has active connections:

   ```sh
   cloudflared tunnel info tunnel-apps
   docker logs cloudflared-apps 2>&1 | grep 'Registered tunnel connection'
   ```

9. Route the retained hostnames and verify the local and public surfaces:

   ```sh
   cloudflared tunnel route dns --overwrite-dns tunnel-apps relay-ai.dongwontuna.net
   cloudflared tunnel route dns --overwrite-dns tunnel-apps nvidia-lb.dongwontuna.net
   cloudflared tunnel route dns --overwrite-dns tunnel-apps orca.dongwontuna.net
   curl -fsS http://127.0.0.1:2455/health
   curl -fsS https://relay-ai.dongwontuna.net/health
   curl -fsS -o /dev/null https://relay-ai.dongwontuna.net/dashboard
   curl -fsS https://nvidia-lb.dongwontuna.net/health/live
   ```

10. Finish with a real Codex response and confirm its matching relay request log
    reports a successful WebSocket upstream.

The retired `${HOME}/.cloudflared/codex-lb.json` credential is not required for
restore unless you are intentionally rolling back the old per-stack tunnel
runner.

## Orca Home

The authoritative install, publication, verification, and rollback commands
are in [`stacks/orca-home/README.md`](../stacks/orca-home/README.md).

1. Restore the shared `tunnel-apps` credential described above. Orca does not
   have a separate Cloudflare credential and must not be moved to the SSH or
   CodexPro tunnel.
2. Run the release-pinned installer with `--activate`. It verifies and extracts
   the AppImage once under `~/.local/orca/releases/`, selects it through
   `~/.local/orca/current`, installs `orca-serve.service`, and verifies the
   private readiness file. The tracked unit intentionally uses
   `AppRun --no-sandbox serve` because this host's AppArmor policy blocks the
   user-namespace sandbox probe; preserve that argument order on restore. Do not
   restore an old pairing offer; Orca regenerates it when the service starts.
3. Confirm user linger, service state, readiness-file mode `0600`, and the local
   `6768` listener without printing the readiness contents.
4. Validate `tunnel-apps`, recreate only `cloudflared-apps`, and wait for active
   tunnel connections before routing `orca.dongwontuna.net`.
5. Verify the public WebSocket `101` upgrade using the secret-free request in
   the Orca README. Never paste the generated pairing link, QR payload, or
   readiness JSON into logs, issues, or pull requests.

The Orca CLI listens on `0.0.0.0:6768`; do not add a router port-forward. The
supported remote path is the Cloudflare hostname.

For a future version rollback, restore the matching Orca profile and release
together. A binary-only rollback is unsafe after a newer profile schema has
been written.

## CodexPro Home

The authoritative install, verification, token-rotation, and rollback commands
are in [`stacks/codexpro-home/README.md`](../stacks/codexpro-home/README.md).

1. Install the documented CodexPro and Cloudflare dependencies, then restore
   `${HOME}/.cloudflared/efdf4f6b-c5ee-4673-b682-eda9a0ef71ca.json` with mode
   `0400`. If that Named Tunnel no longer exists, create a replacement and
   update its ID, credential path, and DNS route together; never reuse the old
   credential with a new tunnel ID.
2. To preserve the existing ChatGPT connector URL, restore the matching
   mode-`0600` file under `${HOME}/.codexpro/profiles/` before saving the
   documented profile settings. Do not pass a replacement `--token`. If the
   profile is unavailable or access should be revoked, let CodexPro create a
   new token and replace the complete connector URL in ChatGPT.
3. Do not restore stale files under `${HOME}/.codexpro/runtime/`.
   `codexpro-home.service` recreates them, and its URL writer recreates
   `${HOME}/.codexpro/current-server-url.txt` with mode `0600`.
4. Install the tracked helper, tunnel config, and user units; validate them,
   reload the user manager, and enable both services exactly as documented.
   Confirm user linger remains enabled so the connector returns after reboot.
5. Before updating ChatGPT, verify the public boundary: only exact `/mcp`
   reaches CodexPro, all other paths return `404`, missing or invalid bearer
   values return `401`, and an authenticated MCP `initialize` succeeds.

Routine rollback disables the tunnel service first and preserves the Named
Tunnel credential, profile token, and DNS route so the same connector URL can
be restored without rotating credentials.
