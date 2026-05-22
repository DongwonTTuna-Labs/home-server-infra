# Home Server Infra

Private operational configuration for DongwonTTuna's home server.

This repository stores reproducible configuration for:

- Forgejo and PostgreSQL
- Forgejo Actions runner
- Cloudflare Tunnel entrypoint
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
stacks/codex-github-runners/  Existing GitHub self-hosted runner pool
stacks/forgejo/               Forgejo service stack and migration helpers
stacks/forgejo-runner/        Forgejo Actions runner stack
```

Application repository workflows are not mirrored here. Each application keeps
its own `.forgejo/workflows` files.

## Quick Checks

```sh
scripts/verify-layout.sh
scripts/scan-secrets.sh
docker compose -f stacks/forgejo/compose.yaml --env-file stacks/forgejo/.env.example config >/dev/null
```

For the Codex GitHub runner stack, `scripts/verify-layout.sh` creates a
temporary placeholder `state/github_pat` outside the tracked tree before running
`docker compose config`.

## Forgejo CLI

`dotfiles/bin/forgejo` is a host wrapper around the running Forgejo container:

```sh
forgejo --version
forgejo admin user list
```

It does not store a token. It uses `docker exec` against `forgejo-forgejo-1`.

