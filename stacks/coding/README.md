# Coding Domain

This directory records the coding/agent container boundary. It intentionally does
not collapse stateful or replica-based services into one container.

## Kept As Domain Stacks

- `stacks/codex-lb`: relay application, Postgres, and update policy. Its
  Cloudflare sidecar is retired into `stacks/tunnel-apps`.
- `stacks/codex-github-runners`: runner pool. Replicas stay separate so jobs do
  not share mutable runner state.
- `/opt/agent-apps`: agent app domain with n8n, OpenClaw, Hermes, and Postgres.
  It remains outside this repository as an operational stack with local state.

## Consolidated Elsewhere

- MCP runtimes are consolidated in `stacks/mcp-suite`.
- Non-SSH Cloudflare connectors are consolidated in `stacks/tunnel-apps`.
- Watchtower is consolidated in `stacks/maintenance` and updates only containers
  explicitly labeled for automatic updates.

## User Systemd Domain

`stacks/coding/systemd/` groups the preserved Codex updater under
`coding-tools.target`:

- `codex-cli-update.timer`

`agentmemory.service`, `web-ai-display.service`, session SSH/GPG agent units,
and local desktop portal masks such as `xdg-desktop-portal*.service` remain
externally owned user/session state. They are intentionally not copied into this
repository because their packages or desktop session own their unit files,
runtime scripts, and mask decisions.

Install or refresh the coding domain units with:

```sh
mkdir -p ~/.config/systemd/user
cp stacks/coding/systemd/*.service ~/.config/systemd/user/
cp stacks/coding/systemd/*.timer ~/.config/systemd/user/
cp stacks/coding/systemd/*.target ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now coding-tools.target codex-cli-update.timer
systemctl --user list-dependencies --plain coding-tools.target
```
