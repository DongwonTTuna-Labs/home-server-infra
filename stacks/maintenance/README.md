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
  tagged releases. Both stay pinned to a release tag plus OCI digest;
  `agent-hermes` is bumped by `hermes-update-latest.timer` below, and
  `agent-openclaw-gateway` by hand.

`codex-lb` is excluded because it is pinned to an operator-verified tag and OCI
digest. Update it manually only after the backup and migration preflight in
`stacks/codex-lb/README.md` succeeds.

A digest-pinned image cannot be updated by Watchtower at all: the reference is
immutable, so the label is silently inert. Dropping the digest is a prerequisite
for enrolling any of these, not an afterthought.

## Hermes Release Updates

`scripts/hermes-update-latest.sh` keeps `agent-hermes` on the newest tagged
`nousresearch/hermes-agent` release — the job Watchtower cannot do, because the
pin is a digest and the only floating tags are branch builds. It resolves the
highest `vYYYY.M.D[.N]` tag, rewrites `HERMES_IMAGE` in `/opt/agent-apps/.env`,
recreates the container, and rolls the pin back if the new release fails to come
up healthy. A release that fails is recorded and skipped until `--force`.

`/opt/agent-apps` is root-owned mode `0640`, so only the pin rewrite runs in a
throwaway container that bind-mounts the stack directory; everything else runs as
the invoking user.

The timer fires at 04:00 KST, clear of both the Hermes cron jobs (08:00-09:00 and
14:00 KST) and Watchtower (09:00 KST), because applying an update restarts the
container. Keep it that way if the schedule is ever changed.

Install:

```sh
install -Dm755 stacks/maintenance/scripts/hermes-update-latest.sh \
  ~/.local/libexec/hermes-update-latest
install -Dm644 stacks/maintenance/systemd/hermes-update-latest.service \
  ~/.config/systemd/user/hermes-update-latest.service
install -Dm644 stacks/maintenance/systemd/hermes-update-latest.timer \
  ~/.config/systemd/user/hermes-update-latest.timer
systemctl --user daemon-reload
systemctl --user enable --now hermes-update-latest.timer
```

Check without changing anything:

```sh
~/.local/libexec/hermes-update-latest --check
systemctl --user list-timers hermes-update-latest.timer
journalctl --user -u hermes-update-latest.service -n 50
```

## n8n Automation Watchdog

`scripts/n8n-workflow-watch.sh` reports n8n automation that has gone quiet.

The failure it exists for is silence, not errors. The Gmail labelling workflow
sat dead for 90 days on an expired OAuth token; that failure happens inside the
polling trigger, which never writes an execution record, so an n8n error
workflow would not have fired once. This checks the observable symptom instead —
active workflows that have never run or have not run inside
`N8N_WATCH_STALE_HOURS` (default 24) — plus failed executions and the
trigger errors n8n only ever writes to its container log.

Alerts go to the `#자동화-오류` Discord channel through Hermes, which already
holds the bot credentials, so no new secret is introduced. Repeat alerts for an
unchanged problem are suppressed for `N8N_WATCH_COOLDOWN_HOURS` (default 12).

Install:

```sh
install -Dm755 stacks/maintenance/scripts/n8n-workflow-watch.sh \
  ~/.local/libexec/n8n-workflow-watch
install -Dm644 stacks/maintenance/systemd/n8n-workflow-watch.service \
  ~/.config/systemd/user/n8n-workflow-watch.service
install -Dm644 stacks/maintenance/systemd/n8n-workflow-watch.timer \
  ~/.config/systemd/user/n8n-workflow-watch.timer
systemctl --user daemon-reload
systemctl --user enable --now n8n-workflow-watch.timer
```

Check without sending anything:

```sh
~/.local/libexec/n8n-workflow-watch --check
```

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
