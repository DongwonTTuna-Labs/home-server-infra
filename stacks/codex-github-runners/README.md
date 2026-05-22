# Codex GitHub self-hosted runners

This stack runs eight organization-level GitHub Actions runners in the
`DongwonTTuna-Labs` organization runner group `Home Server Runners`.

The runner group is configured for all repositories in the organization,
including public repositories. Any `DongwonTTuna-Labs/*` repository can use the
pool by targeting the group and the `codex` label:

```yaml
runs-on:
  group: "Home Server Runners"
  labels: codex
```

The organization sees runner names `codex-01` through `codex-08`. Every runner
has GitHub's default `self-hosted`, `linux`, `x64` labels plus the shared custom
`codex` label and one unique label such as `codex-01`.

Public repositories must keep their fork-PR guard conditions. GitHub warns that
public fork PRs can run untrusted code on self-hosted runners if workflows allow
them through.

## Bootstrap

```bash
cd /home/dongwonttuna/codex-github-runners
cp .env.example .env
mkdir -p state
printf '%s' 'github_pat_xxx' > state/github_pat
chmod 0644 state/github_pat
docker compose build --pull --no-cache
docker compose up -d
chmod 0600 state/github_pat
```

The image installs `@openai/codex@latest`. Use `--pull --no-cache` when
rebuilding so Docker does not reuse an older npm install layer.

The shared image is published locally as `dongwontuna-labs-runner:latest`. It
also includes the latest Node.js 24.x line, Bun, Rust, Cargo, Clippy, Rustfmt,
the native C build toolchain, pkg-config, OpenSSL headers, Python, Git, SSH,
curl, jq, and the Codex auth guard. Forgejo Actions uses the same image through
the `dongwontuna-labs-runner` label, while these legacy GitHub runners keep
their existing `codex` labels for archived GitHub workflows.

The running containers also keep Codex CLI current. `entrypoint.sh` checks npm
before the runner starts, and the pre-job hook checks again before each GitHub
Actions job is released. By default `CODEX_CLI_AUTO_UPDATE=1` and
`CODEX_CLI_VERSION=latest`; set a concrete version only for rollback.

The PAT in `state/github_pat` needs permission to manage self-hosted runners in
the `DongwonTTuna-Labs` organization. The organization currently requires
fine-grained PATs to expire within 366 days. Docker Compose local secrets
preserve the host file mode, so keep it readable during first-time registration.
Once the runner volumes have `.runner` configuration, the stack can restart
without reading the PAT.

To transfer the existing repositories and rewrite their Codex workflows to the
organization runner pool:

```bash
cd /home/dongwonttuna/codex-github-runners
./scripts/migrate-to-org-pool.sh
```

`RUNNER_GROUP_SCOPE=all` is the default because `Home Server Runners` is an
organization-wide group. If the group is later changed to selected repositories,
run the migration with `RUNNER_GROUP_SCOPE=selected` to grant per-repository
access.

## Codex Login Auth

Each runner owns its own `/home/runner/.codex/auth.json` in an independent
Docker volume. Do not copy one `auth.json` across runners: ChatGPT refresh
tokens are single-use, so shared copies race and eventually fail with
`refresh_token_reused`.

To create or verify independent auth for all eight runner volumes:

```bash
cd /home/dongwonttuna/codex-github-runners
./scripts/codex-login-all.sh
```

If a runner already has valid auth, the script skips it. To force fresh device
login for every runner:

```bash
FORCE_CODEX_LOGIN=1 ./scripts/codex-login-all.sh
```

To log in or reseed one runner:

```bash
FORCE_CODEX_LOGIN=1 ./scripts/codex-login-one.sh codex-runner-01
```

To refresh all runner auth volumes manually:

```bash
./scripts/refresh-auth-all.sh
```

To update the Codex CLI in all running runner containers immediately:

```bash
./scripts/update-codex-cli-all.sh
```

To inspect auth without printing secrets:

```bash
./scripts/inspect-auth-all.sh
```

To install the weekly user-level systemd timer:

```bash
./scripts/install-user-timer.sh
```

`sync-auth-to-runners.sh` is retained only as an emergency/debug escape hatch
and refuses to run unless `ALLOW_SHARED_AUTH_SYNC=1` is set.

## Resource Model

The stack keeps eight runners online, and runner pre/post job hooks enforce a
host-wide semaphore with eight slots. Jobs beyond that wait in the runner setup
phase until a slot is free.

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
/home/dongwonttuna/codex-github-runners/state/github_pat
```

It is kept `0600` after registration. This path is also usable for explicit org
runner API checks when needed.

To confirm all runner containers are configured for eight host-wide slots:

```bash
for c in codex-runner-{01..08}; do
  printf '%s ' "$c"
  docker exec "$c" sh -lc 'printf "env=%s runner_env=%s\n" "${CODEX_RUNNER_MAX_PARALLEL:-unset}" "$(sed -n "s/^CODEX_RUNNER_MAX_PARALLEL=//p" /home/runner/actions-runner/.env 2>/dev/null)"'
done
```

To confirm GitHub sees all org runners:

```bash
TOKEN="$(tr -d '\n' < /home/dongwonttuna/codex-github-runners/state/github_pat)"
curl -sS \
  -H "Accept: application/vnd.github+json" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "X-GitHub-Api-Version: 2022-11-28" \
  "https://api.github.com/orgs/DongwonTTuna-Labs/actions/runners?per_page=100"
```
