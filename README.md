# Home Server Infra

Private operational configuration for DongwonTTuna's home server.

This repository stores reproducible configuration for:

- Cloudflare Tunnel entrypoint
- domain-based tunnel suites
- domain-based user systemd bundles
- codex-lb relay
- NVIDIA hosted API load balancer
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
stacks/codexpro-home/         ChatGPT CodexPro home workspace connector
stacks/codex-github-runners/  Existing GitHub self-hosted runner pool
stacks/coding/                Coding/agent domain boundaries
stacks/maintenance/           Single host-wide Watchtower maintenance stack
stacks/nvidia-build-lb/       Independent NVIDIA hosted API gateway stack
stacks/orca-home/             Orca headless remote runtime and pairing service
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
docker compose -f stacks/nvidia-build-lb/compose.yaml config >/dev/null
docker compose -f stacks/maintenance/compose.yaml config >/dev/null
docker compose -f stacks/tunnel-apps/compose.yaml config >/dev/null
node --check stacks/codexpro-home/scripts/codexpro-home-url.mjs
bash -n stacks/orca-home/scripts/*.sh
cloudflared tunnel --config stacks/codexpro-home/cloudflared/codexpro-home.yml ingress validate
systemd-analyze --user verify \
  stacks/codexpro-home/systemd/*.service \
  stacks/orca-home/systemd/*.service \
  stacks/coding/systemd/*.service stacks/coding/systemd/*.timer stacks/coding/systemd/*.target
```

For the Codex GitHub runner stack, `scripts/verify-layout.sh` creates a
temporary placeholder `state/github_pat` outside the tracked tree before running
`docker compose config`.

## User Systemd Domains

User systemd units are grouped like Docker stacks. Update timers use soft
domain targets, while CodexPro uses paired long-running services:

- `coding-tools.target`: `codex-cli-update.timer`
- `codexpro-home.service`: `cloudflared-codexpro-home.service`
- `orca-serve.service`: standalone Orca headless runtime published by `tunnel-apps`

Install or refresh the domain units, then reload user systemd:

```sh
cp stacks/coding/systemd/*.service stacks/coding/systemd/*.timer stacks/coding/systemd/*.target ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now coding-tools.target
```

The CodexPro connector has a dedicated path-restricted Named Tunnel and an
additional mode-`0600` URL writer. Install and verify that bundle using
[`stacks/codexpro-home/README.md`](stacks/codexpro-home/README.md); do not add
its `/mcp` route to `tunnel-apps`.

The Orca runtime intentionally reuses `tunnel-apps` for
`wss://orca.dongwontuna.net`. Install and verify its release-pinned AppImage,
private readiness state, and user service using
[`stacks/orca-home/README.md`](stacks/orca-home/README.md).
