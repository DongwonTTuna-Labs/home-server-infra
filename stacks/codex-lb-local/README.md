# codex-lb-local Stack

Runs `ghcr.io/soju06/codex-lb:latest` on this host as a local relay app/DB
variant. Public routing for `relay-ai.dongwontuna.net` is owned by
`stacks/tunnel-apps`; this stack must not run its own Cloudflare connector.

The single Watchtower instance in `stacks/maintenance` updates the relay image
when this optional stack is deployed. The Postgres service is explicitly
excluded.

## Tracked

- `compose.yaml`

## Host State

Required on this host but not committed:

- `stacks/codex-lb-local/.env` with `CODEX_LB_POSTGRES_PASSWORD`
- Docker volume `codex-lb-local_codex-lb-local-data`
- Docker volume `codex-lb-local_codex-lb-local-postgres-data`

## Deploy

Do not run `stacks/codex-lb` and `stacks/codex-lb-local` simultaneously on the
same host unless one host port is changed; both default to `127.0.0.1:2455`.

```sh
docker compose -f stacks/codex-lb-local/compose.yaml up -d
```

## First-run setup

1. Open http://localhost:2455 (localhost bypasses the dashboard bootstrap token).
2. Add your ChatGPT account (interactive OAuth login).
3. Settings → enable API Key Auth, then API Keys → Create a key with **no
   expiration**. The full key is shown once.
4. Put the key in `~/.codex/ai-relay.env`:
   `export CODEX_LB_LOCAL_API_KEY=sk-clb-...`

## Verify

```sh
curl -fsS http://127.0.0.1:2455/health/ready
```
