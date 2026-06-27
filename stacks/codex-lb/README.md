# codex-lb Stack

This stack owns the Codex relay application and database. Public routing for
`relay-ai.dongwontuna.net` is owned by `stacks/tunnel-apps`.

Image updates are handled by the single Watchtower instance in
`stacks/maintenance` through label-enabled updates. The Postgres service is
explicitly excluded.

## Tracked

- `compose.yaml`

## Host State

These are required on each host but are not committed:

- `stacks/codex-lb/.env` with `CODEX_LB_POSTGRES_PASSWORD`
- Docker volume `codex-lb-data`
- Docker volume `codex-lb_codex-lb-postgres-data`

## Deploy

```sh
docker compose -f stacks/codex-lb/compose.yaml up -d
```

## Verify

```sh
curl -fsS https://relay-ai.dongwontuna.net/health/ready
```
