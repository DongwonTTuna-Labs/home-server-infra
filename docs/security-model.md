# Security Model

The Forgejo service is intended to be reachable only through Cloudflare.

## Public Hostnames

- `git.dongwontuna.net`: Forgejo web UI
- `ssh.dongwontuna.net`: Forgejo SSH Git endpoint

Both hostnames are routed through the existing Cloudflare Tunnel container in
`stacks/agent-stack`.

## Cloudflare Access

The Access application protects both hostnames.

- Action: `Bypass`
- Include: `Gateway = Gateway` OR `Warp = Warp`
- Fallback: owner login can remain enabled for recovery

This means WARP/Gateway-connected devices can reach Forgejo without a browser
login, while non-WARP clients must pass Access login or are blocked.

## Forgejo

- All migrated repositories are private.
- Registration is disabled.
- The admin account has 2FA enabled.
- Forgejo binds only to localhost-published Docker ports on the host:
  - `127.0.0.1:3000` for web
  - `127.0.0.1:2222` for SSH

## Split Tunnel Entries

Cloudflare Zero Trust split tunnel includes:

- `git.dongwontuna.net`
- `ssh.dongwontuna.net`
- `192.168.1.148/32`

