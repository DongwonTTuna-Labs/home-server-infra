# MCP Suite

`mcp-suite` is the single Docker domain for local MCP servers used by OpenCode,
Claude Code, and Codex. It keeps the MCP runtimes containerized while preserving
per-workspace behavior through `docker exec` stdio launchers.

## Servers

| MCP | Runtime | Loopback HTTP | Client-safe stdio launcher |
| --- | --- | --- | --- |
| `lsp` | `oh-my-openagent` LSP daemon | `http://127.0.0.1:8301/mcp` | `docker exec -i -w "$PWD" mcp-suite mcp-suite-stdio lsp` |
| `codegraph` | CodeGraph linux-x64 bundle | `http://127.0.0.1:8302/mcp` | `docker exec -i -w "$PWD" mcp-suite mcp-suite-stdio codegraph` |
| `agbrowse` | `agbrowse web-ai mcp-server` | `http://127.0.0.1:8303/mcp` | `docker exec -i -w "$PWD" mcp-suite mcp-suite-stdio agbrowse` |

Remote MCPs (`websearch`, `context7`, `grep_app`) stay remote. They are not
local stdio runtimes and do not benefit from wrapping in this container.

## Run

```sh
docker compose -f stacks/mcp-suite/compose.yaml up -d --build
docker exec mcp-suite mcp-suite-healthcheck
docker exec mcp-suite mcp-suite-smoke
```

The compose file publishes all MCP HTTP ports on host `127.0.0.1` only. The
proxy binds inside the container on all interfaces so Docker port publishing can
reach it, but the host-side listener remains loopback-only. Do not add these
ports to Cloudflare Tunnel ingress.

For Paca containers, `mcp-suite` also joins the external Docker network
`paca_mcp_internal` as `mcp-suite`. `stacks/paca/compose.yaml` creates that
network; live attach or restart only after Paca has created it. Paca-internal
URLs are:

- `http://mcp-suite:8301/mcp`
- `http://mcp-suite:8302/mcp`
- `http://mcp-suite:8303/mcp`

These URLs are for the private Docker network only. Do not expose them through
Cloudflare Tunnel.

`mcp-suite-smoke` checks HTTP `/ping` readiness and then lists tools through the
same stdio launchers used by clients.

The LSP HTTP proxy runs in stateless mode because the LSP MCP daemon is more
reliable when each Streamable HTTP request gets an isolated stdio session. The
client-facing MCP configs use the stdio launchers above for workspace-sensitive
operations.

## Updates

`mcp-suite` is a local build image, so Watchtower cannot pull new application
bits. Use `mcp-suite-update.timer` to rebuild without cache from latest npm
packages and restart the container. The single maintenance Watchtower only
updates published upstream images selected by label.

Install the user systemd domain bundle with:

```sh
mkdir -p ~/.config/systemd/user ~/.config/mcp-suite
cp stacks/mcp-suite/systemd/mcp-suite.target ~/.config/systemd/user/
cp stacks/mcp-suite/systemd/mcp-suite-update.service ~/.config/systemd/user/
cp stacks/mcp-suite/systemd/mcp-suite-update.timer ~/.config/systemd/user/
cp stacks/mcp-suite/systemd/mcp-suite-update.sh ~/.config/mcp-suite/
chmod 0755 ~/.config/mcp-suite/mcp-suite-update.sh
systemctl --user daemon-reload
systemctl --user enable --now mcp-suite.target mcp-suite-update.timer
systemctl --user list-dependencies --plain mcp-suite.target
```

## Rollback

```sh
systemctl --user disable --now mcp-suite.target mcp-suite-update.timer
docker compose -f stacks/mcp-suite/compose.yaml down
```

Restore client MCP configs from the timestamped backup under
`~/.local/state/home-server-infra/backups/`.
