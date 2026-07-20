# Tunnel Apps

`tunnel-apps` is the single Cloudflare Tunnel domain for non-SSH HTTP apps on
this host. `ssh.dongwontuna.net` remains outside this stack and continues to use
the dedicated SSH tunnel plus `ssh-port-forward`.

## Ingress

| Hostname | Origin |
| --- | --- |
| `relay-ai.dongwontuna.net` | `http://localhost:2455` |
| `paca.dongwontuna.net` | `http://localhost:3080` |
| `nvidia-lb.dongwontuna.net` | `http://localhost:2456` (public dashboard/API/health; admin UI remains loopback-only) |

## Run

Host state required before starting the stack:

- `${HOME}/.cloudflared/685aeec4-5771-459a-8909-7ccfbb086815.json`, mounted
  read-only as the credentials file for tunnel `tunnel-apps`

```sh
cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate
docker compose -f stacks/tunnel-apps/compose.yaml config --quiet
docker compose -f stacks/tunnel-apps/compose.yaml up -d --force-recreate cloudflared-apps
cloudflared tunnel info tunnel-apps
```

Do not move DNS until `tunnel info` reports active connections and the
connector logs contain `Registered tunnel connection` without a subsequent
connection failure.

Move DNS routes only after local origins pass smoke tests:

```sh
cloudflared tunnel route dns --overwrite-dns tunnel-apps relay-ai.dongwontuna.net
cloudflared tunnel route dns --overwrite-dns tunnel-apps paca.dongwontuna.net
cloudflared tunnel route dns --overwrite-dns tunnel-apps nvidia-lb.dongwontuna.net
```

Verify both public routes after the DNS change:

```sh
curl -fsS https://relay-ai.dongwontuna.net/health
curl -fsS -o /dev/null https://relay-ai.dongwontuna.net/dashboard
curl -fsS https://paca.dongwontuna.net/api/healthz
curl -fsS https://nvidia-lb.dongwontuna.net/
curl -fsS https://nvidia-lb.dongwontuna.net/health
```

The previous shared tunnel was deleted and cannot be used as a rollback
target. If this tunnel is revoked or deleted, create another named tunnel,
update its credential mount and tunnel ID together, establish active
connections, and then reroute both DNS records. Do not restore OpenCode DNS or
ingress. Do not stop `cloudflared` from `stacks/agent-stack`; it carries the
SSH tunnel token.

Image updates are handled by the single Watchtower instance in
`stacks/maintenance` through the `cloudflared-apps` update label.
