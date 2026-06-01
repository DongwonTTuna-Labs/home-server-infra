# Codex GitHub self-hosted runners

This stack runs twelve organization-level GitHub Actions runners in the
`DongwonTTuna-Labs` organization runner group `Home Server Runners`.

The runner group is configured for all repositories in the organization,
including public repositories. Any `DongwonTTuna-Labs/*` repository can use the
pool by targeting the group and the `dongwontuna-labs-runner` label:

```yaml
runs-on:
  group: "Home Server Runners"
  labels: dongwontuna-labs-runner
```

The organization sees runner names `home-server-runner-01` through
`home-server-runner-12`. Every runner
has GitHub's default `self-hosted`, `linux`, `x64` labels plus the shared custom
`dongwontuna-labs-runner` label and one unique label such as
`home-server-runner-01`.

Public repositories must keep their fork-PR guard conditions. GitHub warns that
public fork PRs can run untrusted code on self-hosted runners if workflows allow
them through.

## Bootstrap

```bash
cd /home/dongwonttuna/Documents/Programming/home-server-infra/stacks/codex-github-runners
cp .env.example .env
mkdir -p state
printf '%s' 'github_pat_xxx' > state/github_pat
chmod 0644 state/github_pat
docker compose build --pull --no-cache
docker compose up -d
chmod 0600 state/github_pat
```

The shared image is published locally as `dongwontuna-labs-runner:latest`. It
also includes the latest Node.js 24.x line, Codex CLI (`@openai/codex`), Bun,
Rust, Cargo, Clippy, Rustfmt, the native C build toolchain, pkg-config, OpenSSL
headers, Python, Git, SSH, curl, jq, and `lsb_release` from the `lsb-release`
package. GitHub workflows should use the `dongwontuna-labs-runner` label.

The PAT in `state/github_pat` needs permission to manage self-hosted runners in
the `DongwonTTuna-Labs` organization. The organization currently requires
fine-grained PATs to expire within 366 days. Docker Compose local secrets
preserve the host file mode, so keep it readable during first-time registration.
Once the runner volumes have `.runner` configuration, the stack can restart
without reading the PAT.

## Codex OIDC Relay Auth

Codex review workflows authenticate with GitHub OIDC. Workflows should grant
`id-token: write` and use the shared relay setup action to exchange the GitHub
OIDC token with `https://relay-ai.dongwontuna.net/oidc/exchange`.

There is no long-lived `AI_RELAY_API_KEY` organization secret. The workflow may
still set a short-lived exchange result into `AI_RELAY_API_KEY` for the Codex
provider process, but that value is minted per job and is not stored in GitHub
secrets.

## Resource Model

The stack keeps twelve runners online. GitHub schedules jobs across the
organization runner group; this stack does not add a separate local job
semaphore.

Docker resource limits are intentionally modest:

- `mem_limit: 2g`
- `memswap_limit: 2g`
- `cpus: "1.50"`
- `pids_limit: 768`
- `shm_size: 256m`

Docker socket is not mounted into any runner.

## Operational Checks

The org runner API PAT is stored at:

```text
/home/dongwonttuna/Documents/Programming/home-server-infra/stacks/codex-github-runners/state/github_pat
```

It is kept `0600` after registration. This path is also usable for explicit org
runner API checks when needed.

To confirm GitHub sees all org runners:

```bash
TOKEN="$(tr -d '\n' < /home/dongwonttuna/Documents/Programming/home-server-infra/stacks/codex-github-runners/state/github_pat)"
curl -sS \
  -H "Accept: application/vnd.github+json" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  "https://api.github.com/orgs/DongwonTTuna-Labs/actions/runners?per_page=100"
```
