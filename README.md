# Home Server Infra

Private operational configuration for DongwonTTuna's home server.

This repository stores reproducible configuration for:

- Cloudflare Tunnel entrypoint
- codex-lb and GitHub OIDC relay broker
- GitHub Codex runner pool
- Selected SSH and Codex dotfiles

Secrets and runtime data are intentionally excluded. Use the example files and
`docs/secrets.md` to recreate local secret files on a host.

## Layout

```text
docs/                         Operational notes and recovery docs
dotfiles/                     Curated non-secret client config
scripts/                      Repository verification helpers
stacks/agent-stack/           Cloudflare tunnel container stack
stacks/codex-lb/              codex-lb relay and OIDC broker stack
stacks/codex-github-runners/  Existing GitHub self-hosted runner pool
```

Application repository workflows are not mirrored here. Each application keeps
its own GitHub Actions workflows.

## Quick Checks

```sh
scripts/verify-layout.sh
scripts/scan-secrets.sh
docker compose -f stacks/codex-lb/compose.yaml config >/dev/null
```

For the Codex GitHub runner stack, `scripts/verify-layout.sh` creates a
temporary placeholder `state/github_pat` outside the tracked tree before running
`docker compose config`.
