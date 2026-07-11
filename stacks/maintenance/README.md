# Maintenance Stack

This stack owns host-wide container maintenance jobs. It currently runs the
single Watchtower instance for this host.

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

```sh
docker compose -f stacks/maintenance/compose.yaml up -d
```

## Verify

```sh
docker ps --filter 'name=watchtower' --format '{{.Names}}'
docker ps --filter 'label=com.centurylinklabs.watchtower.enable=true' --format '{{.Names}}'
```

The first command should show only `watchtower-maintenance`. The second command
may include Paca only as `ai-agent` and `postgres`, plus non-Paca containers
already allowed by host policy.
