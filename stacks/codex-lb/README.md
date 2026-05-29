# codex-lb Stack

This stack owns the Codex relay, Cloudflare route, GitHub OIDC broker, and
expired-key cleanup timer.

## Tracked

- `compose.yaml`
- `cloudflared/codex-lb.yml`
- `github-oidc-broker/`
- `systemd/github-oidc-broker-cleanup.service`
- `systemd/github-oidc-broker-cleanup.timer`
- `scripts/install-user-timer.sh`

## Host State

These are required on each host but are not committed:

- `${HOME}/.cloudflared/codex-lb.json`
- Docker volume `codex-lb-data`
- Docker volume `github-oidc-broker-data`

## Deploy

```sh
docker compose -f stacks/codex-lb/compose.yaml build codex-lb github-oidc-broker
docker compose -f stacks/codex-lb/compose.yaml up -d
stacks/codex-lb/scripts/install-user-timer.sh
```

## Verify

```sh
curl -fsS http://127.0.0.1:2465/oidc/health
curl -fsS https://relay-ai.dongwontuna.net/oidc/health
docker compose -f stacks/codex-lb/compose.yaml run --rm --no-deps github-oidc-broker python -m app.cleanup_expired_keys --dry-run
systemctl --user list-timers github-oidc-broker-cleanup.timer
```
