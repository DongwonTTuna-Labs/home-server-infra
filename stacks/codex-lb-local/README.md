# codex-lb-local Stack

Runs `ghcr.io/soju06/codex-lb:latest` on this Mac (Docker Desktop) and serves
`relay-ai.dongwontuna.net` through a Cloudflare tunnel created on this host. This
replaces the home-server `codex-lb` stack as the backend for that hostname; the
home-server instance is left running but idle (DNS no longer routes to it).

Watchtower keeps the relay image at latest daily at 09:00 Asia/Seoul.

## Tracked

- `compose.yaml`
- `cloudflared/codex-lb-local.yml`

## Host State

Required on this host but not committed:

- `${HOME}/.cloudflared/bbc484d5-7aa8-4caf-9ec5-15f64c6f5610.json` — tunnel
  credentials for tunnel `codex-lb-local`
- Docker volume `codex-lb-local-data`

## One-time Cloudflare setup

The tunnel and DNS route were created from this host with the account
`cert.pem`:

```sh
cloudflared tunnel create codex-lb-local
cloudflared tunnel route dns --overwrite-dns codex-lb-local relay-ai.dongwontuna.net
```

## Deploy

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
curl -fsS https://relay-ai.dongwontuna.net/health/ready
```
