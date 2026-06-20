# opencode Stack (native)

Runs opencode **natively on the home server** (not in Docker) as user systemd
services, so the agent can read and modify the home server itself. Served at
`opencode.dongwontuna.net` through a dedicated Cloudflare tunnel, mirroring the
laptop opencode setup.

Usable from any device:

- **Browser** (PC / mobile): open `https://opencode.dongwontuna.net`.
- **Terminal TUI**: `opencode attach https://opencode.dongwontuna.net`.

## Why native (not Docker)

The agent runs as the `dongwonttuna` user with `WorkingDirectory=$HOME`, so it can
edit this infra repo, dotfiles, and anything the user can reach — including the
host filesystem (`permission: "allow"`). Runs with no sudo: it uses **user
systemd services** + `loginctl enable-linger` (already on), so it auto-starts on
boot and survives logout/laptop-close.

> Security: the `dongwonttuna` user is in the `docker` group and the agent
> auto-approves all tools, so it effectively has root-equivalent control of the
> host. Access to `opencode.dongwontuna.net` is gated by Cloudflare Access
> (WARP bypass) + HTTP basic auth — keep both in place.

## Components

- `~/.opencode/bin/opencode` — native binary (self-updates via `opencode upgrade`)
- `~/.config/opencode/` — `opencode.json`, `oh-my-openagent.jsonc`, `AGENTS.md`
  (mirrored from `opencode/` here), plus host-only `opencode.env`,
  `cloudflared-opencode.yml`, `opencode-update.sh`
- user units (`~/.config/systemd/user/`):
  - `opencode.service` — `opencode web --hostname 127.0.0.1 --port 4096`
  - `cloudflared-opencode.service` — native cloudflared tunnel → `localhost:4096`
  - `opencode-update.timer` / `opencode-update.service` — 6h idle-aware update

## Config mirror notes

- `oh-my-openagent.jsonc`: sisyphus/prometheus are remapped from
  `anthropic/claude-opus-4-8` to `ai-relay/gpt-5.5` (no Anthropic creds on the
  server). agbrowse MCP and `opencode-claude-auth` are omitted (not portable).

## Auto-update (every 6h)

`opencode-update.timer` runs `opencode-update.sh`: if `/session/status` is `{}`
(idle) it runs `opencode upgrade`, forces `oh-my-openagent@latest` to re-resolve,
and restarts opencode. If busy, it re-checks every 10 min for up to 2h, else
defers to the next cycle — never interrupting a running task.

## Tracked

- `opencode/opencode.json`, `opencode/oh-my-openagent.jsonc`, `opencode/AGENTS.md`
- `systemd/*.service`, `systemd/*.timer`, `systemd/opencode-update.sh`,
  `systemd/cloudflared-opencode.yml`
- `install.sh`, `.env.example`

## Host State (not committed)

- `~/.config/opencode/opencode.env` — `OPENCODE_SERVER_PASSWORD`,
  `CODEX_LB_LOCAL_API_KEY` (copy from `.env.example`)
- `~/.cloudflared/opencode.json` — tunnel credentials for tunnel `opencode`
- `loginctl enable-linger $USER` (already enabled)

## Deploy

```sh
# 1. create secrets
cp stacks/opencode/.env.example ~/.config/opencode/opencode.env
chmod 600 ~/.config/opencode/opencode.env   # then edit real values

# 2. install + start (no sudo)
bash stacks/opencode/install.sh
```

## Verify

```sh
export XDG_RUNTIME_DIR=/run/user/$(id -u)
systemctl --user status opencode cloudflared-opencode opencode-update.timer
curl -fsS -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/global/health
curl -fsS -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/agent   # oh-my-openagent agents
curl -fsS https://opencode.dongwontuna.net/global/health   # via WARP (Access bypass)
```
