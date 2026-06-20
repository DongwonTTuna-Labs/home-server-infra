# opencode Stack

Runs the headless [opencode](https://opencode.ai) server on the home server and
serves it at `opencode.dongwontuna.net` through a dedicated Cloudflare tunnel.

`opencode web` exposes both a browser UI and the HTTP/server API, so you can use
it from any device:

- **Browser** (PC / mobile): open `https://opencode.dongwontuna.net`.
- **Terminal TUI**: `opencode attach https://opencode.dongwontuna.net`.

Because it runs on the home server with `restart: unless-stopped`, it keeps
running when your laptop is closed and comes back automatically after a reboot.

Watchtower keeps the opencode image at latest daily at 09:00 Asia/Seoul. The
cloudflared image is pinned (watchtower disabled for it).

## Tracked

- `compose.yaml`
- `cloudflared/opencode.yml`
- `opencode/opencode.json` — provider/model config (no secrets; the API key is
  injected via `{env:CODEX_LB_LOCAL_API_KEY}`)
- `.env.example`

## Host State

Required on the host but not committed (see `docs/secrets.md`):

- `stacks/opencode/.env` — `OPENCODE_SERVER_PASSWORD`, `CODEX_LB_LOCAL_API_KEY`
  (copy from `.env.example`)
- `${HOME}/.cloudflared/opencode.json` — tunnel credentials for tunnel `opencode`
- Docker volumes `opencode-data` (sessions/auth/state) and `opencode-workspace`

## Security

- `OPENCODE_SERVER_PASSWORD` (HTTP basic auth) is the minimum and is required.
- The opencode server can read/write files and run shell commands, so the public
  hostname **must** sit behind Cloudflare Access (zero-trust). Configure the
  Access application + policy for `opencode.dongwontuna.net` in the Cloudflare
  dashboard (e.g. allow your identity / bypass on WARP). This is managed in
  Cloudflare, not in this repo.
- With Access enabled, the browser uses the Access login page. For
  `opencode attach` through an Access-protected hostname, use a service token
  (`CF-Access-Client-Id` / `CF-Access-Client-Secret`) or `cloudflared access`.

## One-time Cloudflare setup

Run on the host with the account `cert.pem` already present in
`${HOME}/.cloudflared/`:

```sh
# 1. Create the tunnel; this prints a UUID and writes ~/.cloudflared/<UUID>.json
cloudflared tunnel create opencode

# 2. Point the compose-mounted credentials path at that file
ln -sf ~/.cloudflared/<UUID>.json ~/.cloudflared/opencode.json

# 3. Put the UUID into cloudflared/opencode.yml (replace REPLACE_WITH_OPENCODE_TUNNEL_ID)

# 4. Create the public DNS route
cloudflared tunnel route dns opencode opencode.dongwontuna.net
```

Then configure the Cloudflare Access application/policy for the hostname (see
Security above).

## Deploy

```sh
cp stacks/opencode/.env.example stacks/opencode/.env   # then edit real values
docker compose -f stacks/opencode/compose.yaml up -d
```

## Workspace

`opencode-workspace` is a Docker volume mounted at `/workspace` (the opencode
working directory). To work on host files directly instead, replace the
`opencode-workspace:/workspace` volume with a bind mount, e.g.
`/home/dongwonttuna/workspace:/workspace`, and uncomment the `~/.ssh` /
`~/.gitconfig` mounts in `compose.yaml` if opencode needs git credentials.

## Verify

```sh
# On the host (basic auth):
curl -fsS -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/global/health

# Public (after tunnel + Access are configured):
curl -fsS https://opencode.dongwontuna.net/global/health
```
