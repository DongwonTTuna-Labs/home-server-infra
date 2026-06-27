# Tunnel Apps

`tunnel-apps` is the single Cloudflare Tunnel domain for non-SSH HTTP apps on
this host. `ssh.dongwontuna.net` remains outside this stack and continues to use
the dedicated SSH tunnel plus `ssh-port-forward`.

## Ingress

| Hostname | Origin |
| --- | --- |
| `opencode.dongwontuna.net` | `http://localhost:4096` |
| `relay-ai.dongwontuna.net` | `http://localhost:2455` |

## Run

Host state required before starting the stack:

- `${HOME}/.cloudflared/opencode.json`, mounted read-only as the credentials
  file for tunnel `7a1eb69a-992a-4d3d-9eca-bf0e3c4cb916`

```sh
docker compose -f stacks/tunnel-apps/compose.yaml up -d
cloudflared tunnel --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml ingress validate
```

Move DNS routes only after local origins pass smoke tests:

```sh
cloudflared tunnel route dns --overwrite-dns opencode opencode.dongwontuna.net
cloudflared tunnel route dns --overwrite-dns opencode relay-ai.dongwontuna.net
```

After `tunnel-apps` is healthy, retire the old non-SSH tunnel runners:
`cloudflared-opencode.service` and `cloudflared-codex-lb`. Do not stop
`cloudflared` from `stacks/agent-stack`; it carries the SSH tunnel token.

Image updates are handled by the single Watchtower instance in
`stacks/maintenance` through the `cloudflared-apps` update label.
