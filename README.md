# Home Server Infra

Private operational configuration for DongwonTTuna's home server.

This repository stores reproducible configuration for:

- Cloudflare Tunnel entrypoint
- domain-based MCP and tunnel suites
- domain-based user systemd bundles
- codex-lb relay
- GitHub Codex runner pool
- Selected SSH and Codex dotfiles

Secrets and runtime data are intentionally excluded. Use the example files and
`docs/secrets.md` to recreate local secret files on a host.

## Layout

```text
docs/                         Operational notes and recovery docs
dotfiles/                     Curated non-secret client config
scripts/                      Repository verification helpers
stacks/agent-stack/           SSH tunnel container stack
stacks/codex-lb/              codex-lb relay stack
stacks/codex-github-runners/  Existing GitHub self-hosted runner pool
stacks/coding/                Coding/agent domain boundaries
stacks/maintenance/           Single host-wide Watchtower maintenance stack
stacks/mcp-suite/             Single local MCP runtime container
stacks/tunnel-apps/           Single non-SSH Cloudflare Tunnel stack
services/robobotuna-company-os/ Mocked RoboboTuna Company OS first-slice service
```

Application repository workflows are not mirrored here. Each application keeps
its own GitHub Actions workflows.

`services/robobotuna-company-os/` is a local, deterministic, fixture-backed
Company OS implementation boundary. It does not require live Linear, GitHub,
Dify, Grimoire, or production data access.

## Quick Checks

```sh
scripts/verify-layout.sh
scripts/scan-secrets.sh
CODEX_LB_POSTGRES_PASSWORD=placeholder docker compose -f stacks/codex-lb/compose.yaml config >/dev/null
docker compose -f stacks/maintenance/compose.yaml config >/dev/null
docker compose -f stacks/mcp-suite/compose.yaml config >/dev/null
docker compose -f stacks/tunnel-apps/compose.yaml config >/dev/null
systemd-analyze --user verify \
  stacks/mcp-suite/systemd/*.service stacks/mcp-suite/systemd/*.timer stacks/mcp-suite/systemd/*.target \
  stacks/coding/systemd/*.service stacks/coding/systemd/*.timer stacks/coding/systemd/*.target
```

For the Codex GitHub runner stack, `scripts/verify-layout.sh` creates a
temporary placeholder `state/github_pat` outside the tracked tree before running
`docker compose config`.

## User Systemd Domains

User systemd units are grouped like Docker stacks with soft domain targets:

- `mcp-suite.target`: `mcp-suite-update.timer`
- `coding-tools.target`: `codex-cli-update.timer`

Install or refresh the domain units, then reload user systemd:

```sh
cp stacks/mcp-suite/systemd/*.service stacks/mcp-suite/systemd/*.timer stacks/mcp-suite/systemd/*.target ~/.config/systemd/user/
cp stacks/coding/systemd/*.service stacks/coding/systemd/*.timer stacks/coding/systemd/*.target ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now mcp-suite.target coding-tools.target
```
