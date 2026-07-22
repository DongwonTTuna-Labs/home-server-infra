# Maintenance Stack

This stack owns host-wide container maintenance jobs. It runs the single
Watchtower instance for this host and the host-wide Docker prune timer.

## Watchtower Policy

Watchtower runs in label-enable mode, so only containers with
`com.centurylinklabs.watchtower.enable=true` are updated. Stateful databases,
runner pools, local-build images, and SSH tunnel infrastructure must stay
unlabeled or explicitly set to `false`.

Current intended update targets:

- `cloudflared-apps`

`codex-lb` is excluded because it is pinned to an operator-verified tag and OCI
digest. Update it manually only after the backup and migration preflight in
`stacks/codex-lb/README.md` succeeds.

`mcp-suite` is excluded because it is a local build image; updates are handled by
`mcp-suite-update.timer`.

## Docker Prune Policy

`WATCHTOWER_CLEANUP=true` only removes the *old* image of a container Watchtower
itself updates, and Watchtower only touches label-enabled containers. Everything
outside that scope accumulates until the root disk fills:

- local-build images (`mcp-suite`, `gpt-webai-slot`) rebuilt on their own timers,
- superseded tags from manual pulls (`postgres`, `cloudflared`, `nvidia-build-lb`),
- BuildKit cache from `mcp-suite-update.timer` (rebuilds without cache every 6h),
- stale stopped containers.

`docker-prune.timer` is the host-wide daily sweep that covers all of the above.
It runs, in order:

```sh
docker container prune -f --filter until=24h   # stopped >24h only
docker image prune -af                          # images with no container ref
docker builder prune -af                        # BuildKit cache
```

Running and stopped containers always keep their own images, so active stacks
are never affected.

### Volumes are excluded on purpose

The prune job never runs `docker volume prune`. Named data volumes
(`codex-lb_*`, `paca_*`, `nvidia-build-lb_*`, relay-ai postgres) must never be
auto-removed. Anonymous volume churn — mostly from the GitHub runner pool — is
reclaimed **manually** and only after confirming no named data volume is caught:

```sh
# preview named dangling volumes (inspect before removing any)
docker volume ls -qf dangling=true | grep -vE '^[0-9a-f]{64}$'
# remove only anonymous (64-hex) dangling volumes
docker volume ls -qf dangling=true | grep -E '^[0-9a-f]{64}$' | xargs -r docker volume rm
```

## Paca Exceptions

Paca `ai-agent` and Paca `postgres` are the only user-approved high-risk Paca
auto-update exceptions. This is not the default policy for app workers or
stateful databases.

| Paca service | `com.centurylinklabs.watchtower.enable` | Policy |
| --- | --- | --- |
| `ai-agent` | `true` | User-approved exception. Requires xhigh drift detection and rollback evidence. |
| `postgres` | `true` | User-approved exception. Requires current backup, DB health check, and rollback evidence. |
| `api` | `false` or absent | Not approved for auto-update. |
| `web` | `false` or absent | Not approved for auto-update. |
| `realtime` | `false` or absent | Not approved for auto-update. |
| `gateway` | `false` or absent | Not approved for auto-update. |
| `minio` | `false` or absent | Not approved for auto-update. |
| `valkey` | `false` or absent | Not approved for auto-update. |
| `db-backup` | `false` or absent | Not approved for auto-update. |

Watchtower hooks can't block a bad update after it starts. They can only detect
and log. For Paca `ai-agent` and `postgres`, keep detection output, backup path,
prior image refs, and rollback smoke results under `.omo/evidence/`. That path
is local-only and ignored.

Paca rollback docs live in `docs/restore.md`. Any Postgres rollback needs an
explicit backup file and user approval before data is replaced.

## Run

Watchtower container:

```sh
docker compose -f stacks/maintenance/compose.yaml up -d
```

Host-wide Docker prune timer (user systemd):

```sh
cp stacks/maintenance/systemd/docker-prune.service ~/.config/systemd/user/
cp stacks/maintenance/systemd/docker-prune.timer ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now docker-prune.timer
```

## Verify

```sh
docker ps --filter 'name=watchtower' --format '{{.Names}}'
docker ps --filter 'label=com.centurylinklabs.watchtower.enable=true' --format '{{.Names}}'
systemctl --user list-timers docker-prune.timer --no-pager
systemctl --user start docker-prune.service   # optional: run once now
```

The first command should show only `watchtower-maintenance`. The second command
may include Paca only as `ai-agent` and `postgres`, plus non-Paca containers
already allowed by host policy. `list-timers` should show `docker-prune.timer`
scheduled for the next 04:00 UTC.
