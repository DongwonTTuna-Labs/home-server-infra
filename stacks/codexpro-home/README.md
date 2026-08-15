# CodexPro Home

`codexpro-home` exposes the user-approved `/home/dongwonttuna` workspace to a
single ChatGPT connector through CodexPro and a dedicated Cloudflare Named
Tunnel. The public hostname is `codexpro.dongwontuna.net`.

This stack intentionally remains separate from `stacks/tunnel-apps`:

- The retired local `mcp-suite` must not be restored or published.
- `tunnel-apps` serves ordinary HTTP applications and rejects MCP ingress.
- `codexpro-home` has its own tunnel credential and publishes exactly `/mcp`.

## Security boundary

The deployed CodexPro profile is intentionally broad because the connector is
used across the home workspace:

| Setting | Value |
| --- | --- |
| Root | `/home/dongwonttuna` |
| Mode | `agent` |
| Write | `workspace` |
| Bash | `safe` |
| Tool mode | `full` |
| Codex session reads | `off` |

CodexPro's built-in sensitive-path guards are extended with blocks for local
Codex, Cloudflare, GnuPG, cloud-provider, container, Git hosting, keyring, and
common secret-store paths. These guards reduce exposure but are not an OS
sandbox.

The public route is narrower than the local server:

- `https://codexpro.dongwontuna.net/mcp` is forwarded to loopback port `8788`.
- Every other hostname/path match receives `404` at Cloudflare Tunnel ingress.
- `/mcp` still requires the saved 48-character CodexPro bearer token.

On the host, the token is stored only in mode-`0600` CodexPro profile/runtime
state and the generated connector URL. Never commit, log, screenshot, or paste
that full URL into an issue or pull request.

## Why the services are split

CodexPro's integrated Named Tunnel mode waits for a public `/healthz` response
during startup. Publishing that path would violate the `/mcp`-only ingress
contract. The tracked deployment therefore runs:

1. `codexpro-home.service`: loopback-only CodexPro on `127.0.0.1:8788`.
2. `cloudflared-codexpro-home.service`: the dedicated Named Tunnel with the
   path-restricted ingress file and metrics pinned to `127.0.0.1:20242`.

The explicit metrics port prevents a boot-order race with the SSH tunnel,
which reserves `127.0.0.1:20241`. Without this pin, `cloudflared` may claim the
SSH port during reboot and leave the SSH connector in a restart loop.

CodexPro prints its token-bearing local URL on standard output. The service
discards standard output while retaining standard error in the user journal.
`codexpro-home-url` writes the stable connector URL directly to a mode-`0600`
file instead. The helper requires exactly one of `--write` or `--redacted` and
never prints the bearer URL by default.

## Prerequisites

The verified host versions are CodexPro `0.29.0`, Node.js `24`, and
`cloudflared` `2026.5.0`. Install CodexPro without root privileges:

```bash
npm install --global --prefix "$HOME/.local" codexpro@0.29.0
```

The Cloudflare tunnel already exists with these non-secret identifiers:

- name: `codexpro-home`
- ID: `efdf4f6b-c5ee-4673-b682-eda9a0ef71ca`
- hostname: `codexpro.dongwontuna.net`

Its credential is host-only state and is not tracked:

```text
~/.cloudflared/efdf4f6b-c5ee-4673-b682-eda9a0ef71ca.json
```

Keep that file mode `0400`. If the tunnel must be recreated, update the tunnel
ID and credential path together in `cloudflared/codexpro-home.yml`, then replace
the DNS route only after the new tunnel reports active connections.

## Save the CodexPro profile

This command preserves an existing CodexPro token. On a clean host, CodexPro
creates a new private token in its mode-`0600` workspace profile.

```bash
codexpro settings set \
  --root "$HOME" \
  --port 8788 \
  --mode agent \
  --write workspace \
  --bash safe \
  --tool-mode full \
  --tunnel cloudflare-named \
  --hostname codexpro.dongwontuna.net \
  --tunnel-name codexpro-home \
  --cloudflare-config "$HOME/.cloudflared/codexpro-home.yml" \
  --no-install-cloudflared
```

Do not pass `--token` during routine reinstalls. Keeping the saved token stable
keeps the registered ChatGPT connector URL valid.

## Install or refresh

Run from the canonical checkout at
`$HOME/Documents/Programming/home-server-infra`:

```bash
set -euo pipefail

stack=stacks/codexpro-home
install -d -m 0700 "$HOME/.cloudflared" "$HOME/.codexpro"
install -d -m 0755 "$HOME/.local/bin" "$HOME/.config/systemd/user"
install -D -m 0600 \
  "$stack/cloudflared/codexpro-home.yml" \
  "$HOME/.cloudflared/codexpro-home.yml"
install -D -m 0755 \
  "$stack/scripts/codexpro-home-url.mjs" \
  "$HOME/.local/bin/codexpro-home-url"
install -D -m 0644 \
  "$stack/systemd/codexpro-home.service" \
  "$HOME/.config/systemd/user/codexpro-home.service"
install -D -m 0644 \
  "$stack/systemd/cloudflared-codexpro-home.service" \
  "$HOME/.config/systemd/user/cloudflared-codexpro-home.service"

node --check "$HOME/.local/bin/codexpro-home-url"
cloudflared tunnel \
  --config "$HOME/.cloudflared/codexpro-home.yml" \
  ingress validate
systemd-analyze --user verify \
  "$HOME/.config/systemd/user/codexpro-home.service" \
  "$HOME/.config/systemd/user/cloudflared-codexpro-home.service"

systemctl --user daemon-reload
systemctl --user enable --now \
  codexpro-home.service cloudflared-codexpro-home.service
```

Both services are enabled under `default.target`. The host also has user linger
enabled, so they start after reboot without an interactive login. Verify this
without changing it:

```bash
loginctl show-user "$USER" -p Linger
```

## ChatGPT connector

Read the full private connector URL only from a local terminal:

```bash
cat "$HOME/.codexpro/current-server-url.txt"
```

In ChatGPT, create or update the connector with:

- connection: `Server URL`
- server URL: the complete line from `current-server-url.txt`
- authentication: `No Authentication / None`

Do not use the OAuth advanced form. CodexPro `0.29.0` does not publish OAuth
protected-resource metadata, an authorization server, DCR, or CIMD. ChatGPT's
`token endpoint authentication method: none` is still part of an OAuth flow;
it is not equivalent to the connector's `No Authentication` choice. CodexPro
authenticates the URL token itself.

## Verification

Validate the tracked files before deployment:

```bash
node --check stacks/codexpro-home/scripts/codexpro-home-url.mjs
cloudflared tunnel \
  --config stacks/codexpro-home/cloudflared/codexpro-home.yml \
  ingress validate
cloudflared tunnel \
  --config stacks/codexpro-home/cloudflared/codexpro-home.yml \
  ingress rule https://codexpro.dongwontuna.net/mcp
cloudflared tunnel \
  --config stacks/codexpro-home/cloudflared/codexpro-home.yml \
  ingress rule https://codexpro.dongwontuna.net/healthz
systemd-analyze --user verify stacks/codexpro-home/systemd/*.service
```

The first ingress rule query must match the loopback origin; the second must
match the `http_status:404` catch-all.

Check the live boundary:

```bash
set -euo pipefail

base=https://codexpro.dongwontuna.net
test "$(curl -sS -o /dev/null -w '%{http_code}' "$base/")" = 404
test "$(curl -sS -o /dev/null -w '%{http_code}' "$base/healthz")" = 404
test "$(curl -sS -o /dev/null -w '%{http_code}' "$base/admin/profile")" = 404
test "$(curl -sS -o /dev/null -w '%{http_code}' "$base/mcp/")" = 404
test "$(curl -sS -o /dev/null -w '%{http_code}' -X POST "$base/mcp")" = 401

server_url=$(<"$HOME/.codexpro/current-server-url.txt")
printf 'url = "%s"\n' "$server_url" | curl --config - -fsS \
  -X POST \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  -H 'mcp-protocol-version: 2025-03-26' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"infra-smoke","version":"1.0"}}}' \
  | grep -F '"protocolVersion":"2025-03-26"'

systemctl --user is-enabled codexpro-home.service
systemctl --user is-active codexpro-home.service
systemctl --user is-enabled cloudflared-codexpro-home.service
systemctl --user is-active cloudflared-codexpro-home.service
cloudflared tunnel info codexpro-home
```

## Token rotation

Rotating the CodexPro token is equivalent to changing a password. It creates a
new connector URL and makes the old URL return `401`. Rotate only when the URL
may have leaked or access must be revoked, then restart `codexpro-home.service`
and replace the complete URL in ChatGPT. A hostname or tunnel restart does not
require token rotation.

## Rollback

Disable public access without deleting credentials or DNS:

```bash
systemctl --user disable --now cloudflared-codexpro-home.service
```

Disable the local connector as well:

```bash
systemctl --user disable --now codexpro-home.service
```

Do not delete the Named Tunnel or its credential during routine rollback. Both
services can be restored from the tracked files and re-enabled without changing
the ChatGPT URL.
