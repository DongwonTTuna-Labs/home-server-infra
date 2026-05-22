# GitHub to Forgejo Migration Kit

This directory implements the local side of the migration plan for:

- `bioden`
- `polymarket-liquidity-farming-rs`
- `rs-builder-relayer-client`

The current repo state is not freeze-ready yet, so the destructive cutover steps are scripts you run after all PRs and local work are merged to `main`.

## 1. Configure Forgejo

```sh
cd /home/dongwonttuna/Documents/Programming/forgejo-migration
cp .env.example .env
openssl rand -hex 32
```

Put the generated value in `POSTGRES_PASSWORD`, then adjust `FORGEJO_DOMAIN`, `FORGEJO_SSH_DOMAIN`, `FORGEJO_ROOT_URL`, `FORGEJO_API_URL`, and `FORGEJO_SSH_BASE` if the Git domain is not `git.dongwontuna.net`.

Start Forgejo:

```sh
docker compose --env-file .env -f compose.yaml up -d
docker compose --env-file .env -f compose.yaml logs -f forgejo
```

If Docker created `./forgejo` as root before the first start, fix bind-mount ownership with:

```sh
scripts/prepare-dirs.sh
docker compose --env-file .env -f compose.yaml up -d
```

Complete the first web login at `http://127.0.0.1:3000` or through the Cloudflare hostname. Use `DongwonTTuna` as the initial admin username. The scripts migrate repositories into the `DongwonTTuna-Labs` organization while still reading the source repositories from GitHub `DongwonTTuna-Labs`.

`DongwonTTuna-Labs` exists as the migration organization. All migrated repositories are private, and web/SSH access is expected to sit behind Cloudflare Access.

## 2. Configure Cloudflare Tunnel

This host already runs a token-based `cloudflared` service from `/opt/agent-stack/compose.yml`:

- container: `cloudflared`
- compose project: `agent-stack`
- network mode: `host`
- tunnel: `SSH` / `1482bc47-42df-4c24-8cf3-52fd64f49336`

Do not start a second `cloudflared` for Forgejo. The existing container can reach Forgejo through the host-published localhost ports:

- `git.dongwontuna.net` to `http://localhost:3000`
- `ssh.dongwontuna.net` to `ssh://localhost:2222`

The DNS CNAME routes for both hostnames are created with `cloudflared tunnel route dns`. The remote tunnel ingress config has also been updated through the Cloudflare tunnel config API:

- `n8n.dongwontuna.net` remains routed to `http://127.0.0.1:5678`
- `git.dongwontuna.net` is routed to `http://127.0.0.1:3000`
- `ssh.dongwontuna.net` is routed to `ssh://127.0.0.1:2222`
- unmatched hostnames return `http_status:404`

The provided origin certificate can update tunnel ingress, but it cannot manage Cloudflare Access applications. Create or verify a Cloudflare Access application covering both `git.dongwontuna.net` and `ssh.dongwontuna.net`.

The server also has the Cloudflare One Client package installed for local testing. Because Ubuntu 26.04 does not yet have a `resolute` Cloudflare WARP apt repo, the `noble` repo is configured in `/etc/apt/sources.list.d/cloudflare-client.list`. The device is enrolled in the `dongwontuna` Zero Trust organization and `warp-cli status` should report `Connected`.

The current security model is:

- `git.dongwontuna.net` and `ssh.dongwontuna.net` are protected by Cloudflare Access.
- WARP/Gateway-connected devices can bypass the Access login screen.
- Login fallback can remain enabled for owner recovery, but repositories themselves stay private in Forgejo.

Use `cloudflared/ssh-config.snippet` in `~/.ssh/config` for Git SSH through Cloudflare Access.

## 3. Register an Actions Runner

Forgejo Actions need a runner. Generate a 40-character hex secret:

```sh
openssl rand -hex 20
```

Put it in `.env` as `FORGEJO_RUNNER_SECRET`, then run:

```sh
scripts/register-runner.sh
docker compose --env-file .env -f runner.compose.yaml up -d
```

The default labels are:

- `ubuntu-latest:docker://node:24-bookworm`
- `rust:docker://rust:1-bookworm`

## 4. Freeze and Back Up

After every branch/PR/local edit is merged to `main`, run:

```sh
scripts/freeze-audit.sh
scripts/backup-bundles.sh
```

This stores GitHub metadata under `metadata/<timestamp>/` and Git bundles under `backups/<timestamp>/`.

## 5. Migrate Repositories

Preferred path: use Forgejo's GitHub migration UI/API so PR metadata is imported.

API path:

```sh
scripts/migrate-via-forgejo-api.sh
```

Fallback mirror path:

```sh
scripts/push-mirrors.sh
scripts/switch-remotes.sh
scripts/verify-forgejo.sh
```

The fallback preserves Git branches/tags but not PR discussion metadata.

Current Forgejo state:

- `bioden` exists on Forgejo as a private repository, with GitHub `origin/*` branches and tags pushed.
- `polymarket-liquidity-farming-rs` exists on Forgejo as a private repository, with GitHub `origin/*` branches pushed.
- `rs-builder-relayer-client` exists on Forgejo as a private repository, with GitHub `origin/*` branches pushed.
- Local Git remotes still keep `origin` pointed at GitHub; a non-destructive `forgejo` remote points to `https://git.dongwontuna.net/DongwonTTuna-Labs/<repo>.git`.
- The local `.forgejo/` workflow files are still uncommitted in each repository and have not been pushed.

## 6. After Cutover

When Forgejo clone/push, PR creation, Actions, and `bioden` staging deploy all pass:

```sh
scripts/archive-github.sh --dry-run
scripts/archive-github.sh
```

Keep the GitHub repos archived/read-only for at least two weeks.

## Forgejo Secrets

Add these secrets after each repository is migrated:

- `bioden`: `CLOUDFLARE_API_TOKEN`, `FORGEJO_BOT_TOKEN`
- `polymarket-liquidity-farming-rs`: `FORGEJO_BOT_TOKEN`
- `rs-builder-relayer-client`: `FORGEJO_BOT_TOKEN`

Optional Codex review execution can be enabled by adding `CODEX_REVIEW_COMMAND` as a variable or secret. The command receives a PR review prompt on stdin and should print Markdown on stdout. If it is absent, the Forgejo review workflow still updates the sticky comment with collected PR metadata and diff stats, which verifies the Forgejo API/token path before plugging in the actual reviewer.
