# opencode Stack

Runs an opencode server on the home server, served at `opencode.dongwontuna.net`
through a dedicated Cloudflare tunnel, mirroring the laptop opencode setup.

`opencode web` exposes a browser UI and the HTTP API, usable from any device:

- **Browser** (PC / mobile): open `https://opencode.dongwontuna.net`.
- **Terminal TUI**: `opencode attach https://opencode.dongwontuna.net`.

Runs with `restart: unless-stopped`, so it survives laptop close and reboots.

## Image

The official `ghcr.io/anomalyco/opencode` image is a minimal runtime (no git /
ssh / node). This stack builds its own dev image (`Dockerfile`) instead:

- base `node:22-bookworm-slim` + opencode (official install script)
- `git`, `openssh-client`, `curl`, `ca-certificates`, `ripgrep`, `tmux`, `jq`

## Mirrored config

`opencode/` is mounted read-only into the container config and mirrors the laptop:

- `opencode.json` — `ai-relay` provider (relay-ai.dongwontuna.net), `gpt-5.5`
  default, `oh-my-openagent` plugin, `permission: "allow"` (auto-approve)
- `oh-my-openagent.jsonc` — agent/category model map; **sisyphus/prometheus are
  remapped from `anthropic/claude-opus-4-8` to `ai-relay/gpt-5.5`** because the
  server has no Anthropic credentials (add a key + restore to use claude-opus)
- `AGENTS.md` — global prompt

Differences from the laptop (intentional):

- `agbrowse` MCP is omitted (it needs the laptop's node path + ChatGPT browser
  session; not portable to a headless container).
- `opencode-claude-auth` plugin is omitted (no Anthropic auth on the server).

## Auto-update (every 6h)

`opencode-updater` (docker:cli + docker socket) runs every 6h:

1. `opencode upgrade` (self-update the binary)
2. force `oh-my-openagent@latest` to re-resolve (clears the plugin cache)
3. restart opencode to apply

## Tracked

- `Dockerfile`, `compose.yaml`, `cloudflared/opencode.yml`
- `opencode/opencode.json`, `opencode/oh-my-openagent.jsonc`, `opencode/AGENTS.md`
- `.env.example`

## Host State

Required on the host but not committed (see `docs/secrets.md`):

- `stacks/opencode/.env` — `OPENCODE_SERVER_PASSWORD`, `CODEX_LB_LOCAL_API_KEY`
- `${HOME}/.cloudflared/opencode.json` — tunnel credentials for tunnel `opencode`
- Docker volumes `opencode-config` (plugins), `opencode-cache`, `opencode-data`
  (sessions/auth), `opencode-workspace`

## Security

- `OPENCODE_SERVER_PASSWORD` (HTTP basic auth) is required.
- `permission: "allow"` auto-approves every tool call, and the agent runs as root
  with full shell access. The public hostname **must** sit behind Cloudflare
  Access (zero-trust); configure the Access app/policy (e.g. WARP bypass) in the
  Cloudflare dashboard. Managed in Cloudflare, not this repo.

## Git auth (optional, not enabled)

The image has `git`/`ssh`, but no credentials are mounted, so only public repos
work. To clone/push private repos, uncomment the `~/.ssh` / `~/.gitconfig` mounts
in `compose.yaml` (provides the host key to a root, auto-approving agent — enable
Cloudflare Access first).

## One-time Cloudflare setup

```sh
cloudflared tunnel create opencode
ln -sf ~/.cloudflared/<UUID>.json ~/.cloudflared/opencode.json
# put <UUID> into cloudflared/opencode.yml
cloudflared tunnel route dns opencode opencode.dongwontuna.net
```

## Deploy

```sh
cp stacks/opencode/.env.example stacks/opencode/.env   # then edit real values
docker compose -f stacks/opencode/compose.yaml up -d --build
```

## Verify

```sh
curl -fsS -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/global/health
curl -fsS -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/agent   # oh-my-openagent agents
curl -fsS https://opencode.dongwontuna.net/global/health
```
