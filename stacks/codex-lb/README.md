# codex-lb Stack

This stack owns the Codex relay, native OIDC token exchange, and Cloudflare
route.

## Tracked

- `compose.yaml`
- `cloudflared/codex-lb.yml`

## Host State

These are required on each host but are not committed:

- `${HOME}/.cloudflared/codex-lb.json`
- Docker volume `codex-lb-data`

## Deploy

```sh
docker compose -f stacks/codex-lb/compose.yaml build codex-lb
docker compose -f stacks/codex-lb/compose.yaml up -d
```

## Verify

```sh
curl -fsS https://relay-ai.dongwontuna.net/health/ready
curl -fsS https://relay-ai.dongwontuna.net/oidc/health
```
