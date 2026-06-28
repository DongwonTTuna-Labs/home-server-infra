# Maintenance Stack

This stack owns host-wide container maintenance jobs. It currently runs the
single Watchtower instance for this host.

## Watchtower Policy

Watchtower runs in label-enable mode, so only containers with
`com.centurylinklabs.watchtower.enable=true` are updated. Stateful databases,
runner pools, local-build images, and SSH tunnel infrastructure must stay
unlabeled or explicitly set to `false`.

Current intended update targets:

- `codex-lb`
- `cloudflared-apps`

`mcp-suite` is excluded because it is a local build image; updates are handled by
`mcp-suite-update.timer`.

## Run

```sh
docker compose -f stacks/maintenance/compose.yaml up -d
```

## Verify

```sh
docker ps --filter 'name=watchtower' --format '{{.Names}}'
docker ps --filter 'label=com.centurylinklabs.watchtower.enable=true' --format '{{.Names}}'
```

The first command should show only `watchtower-maintenance`.
