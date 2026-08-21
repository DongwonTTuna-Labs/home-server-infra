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
- `agent-n8n`

`agent-n8n` follows the vendor's `stable` channel rather than a digest, so
Watchtower can roll it. n8n applies one-way database migrations on startup, so
keep a current `pg_dump` of the `n8n` database under `~/backups/agent-apps/`;
there is no supported downgrade once a release lands.

The rest of `/opt/agent-apps` stays excluded, for two different reasons:

- `agent-postgres` is a stateful database.
- `agent-hermes` and `agent-openclaw-gateway` publish no floating tag that
  tracks releases. For both images `latest` resolves to the same digest as
  `main`, so labelling them would follow unreleased branch builds rather than
  tagged releases. They stay pinned to a release tag plus OCI digest and are
  bumped by hand.

`codex-lb` is excluded because it is pinned to an operator-verified tag and OCI
digest. Update it manually only after the backup and migration preflight in
`stacks/codex-lb/README.md` succeeds.

A digest-pinned image cannot be updated by Watchtower at all: the reference is
immutable, so the label is silently inert. Dropping the digest is a prerequisite
for enrolling any of these, not an afterthought.

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
may include only containers already allowed by host policy.
