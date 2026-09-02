# ADR: Take System DNS Away From Cloudflare WARP

## Status

Accepted and applied on 2026-08-26. Verified live on `dongwontuna-net-server`: WARP reports
`Mode: TunnelOnly`, systemd-resolved has no WARP-owned resolvers, and both codex-lb request
paths pass an end-to-end smoke.

## Context

Cloudflare WARP runs always-on on this host under a Zero Trust device profile. In
`WarpWithDnsOverHttps` mode the client takes ownership of system DNS two ways at once:

- it registers a `CloudflareWARP` link in systemd-resolved carrying the catch-all routing
  domain `~.`, which outranks every other link, and
- it sets the resolved *global* DNS servers to its local DoH proxy, `127.0.2.2` / `127.0.2.3`.

The working LAN resolvers advertised by DHCP on `enp2s0` (`172.20.11.10`, `172.20.11.11`)
were therefore never consulted. WARP became a single point of failure for all name
resolution on the box.

That failure arrived on 2026-08-26 at 06:56:02Z. WARP proxies DoH to
`208eb6b3e95bca63afa92342baa60654.cloudflare-gateway.com` at `162.159.36.1` /
`162.159.46.1`, and those anycast addresses stopped answering from this uplink — TCP 53, 80,
and 443 all time out, sustained across the whole outage, while the DHCP-supplied LAN
resolvers stay reachable throughout. The route is a direct one via the LAN gateway, not
through the tunnel, so the break is upstream of this host and not ours to fix. warp-svc
logged the cause plainly:

```
WARN dns_proxy::resolver::errors: DoH error encountered
     error=ProtoError { kind: Timeout } ip=162.159.36.1 error_source=Establishment
DEBUG dns_proxy::errors: DnsProxy timeout target=chatgpt.com.
```

The prior DoH connection had been healthy since 2026-08-25 14:13Z
(`attempted_queries:9991, successful_queries:9989`), so nothing local had changed.

Both addresses became reachable again about an hour later, at roughly 07:50Z, with
`162.159.46.1` still handshaking slowly at 2.09s. The outage was transient, which is the
argument for this ADR rather than against it: the same interruption will recur, and the
structure below decides whether it takes the whole host down with it.

Host DNS death propagated straight into Docker: the embedded resolver at `127.0.0.11`
forwards to the host stub, so every container inherited the outage. codex-lb surfaced it as
`socket.gaierror: [Errno -3] Temporary failure in name resolution` wrapped in
`ClientConnectorDNSError`, and every streaming request ended `code=upstream_unavailable`.
`warp-cli status` kept reporting `Connected / Network: healthy` throughout, so the client
status is not a usable signal for this failure.

Local remediation was blocked: `warp-cli mode tunnel_only` returns
`Error: Operation not authorized in this context.` because the mode is owned by the device
profile, and `sudo` does not change that. The fix had to happen in Zero Trust.

## Decision

DNS resolution belongs to the operating system. WARP keeps the tunnel and nothing else.

A dedicated Zero Trust device settings profile now scopes that change to this host alone:

| Field | Value |
| --- | --- |
| Name | `linux-home-server-traffic-only` |
| Profile ID | `bd2d788f-694b-4c90-bec7-12f8b8431d43` |
| Expression | `os.name is linux` |
| Precedence | 1, above `Default` |
| Service mode | Traffic only mode (`warp_tunnel_only`, client `TunnelOnly`) |
| Split tunnel | Include, inherited from `Default` |

Linux uniquely identifies this box. The other three enrolled devices are macOS and mobile
and stay on `Default`, so the Mac keeps Gateway DNS filtering and its host-based split
tunnel entries. Device profiles cannot select on hostname or serial, so OS is the narrowest
available selector.

The profile was created matching `os.name is windows` first — a deliberate no-match, since
no Windows device is enrolled — so the split tunnel could be reviewed before any device
picked the profile up. Only then was the expression flipped to `linux`.

## Consequences

Verified after rollout:

- `resolvectl`: global DNS list empty, `CloudflareWARP` link at `Current Scopes: none` and
  `Default Route: no`, `enp2s0` serving `172.20.11.10` / `172.20.11.11`.
- The tunnel survives. `ip route get 192.168.1.148` and `ip route get 172.64.128.1` both
  resolve `dev CloudflareWARP`, so IP-based include entries still route through WARP.
- codex-lb resolves upstreams and completes streams; zero DNS errors since restart.

Accepted trade-offs:

- **Host-based split tunnel entries are inert on this host.** `relay-ai.dongwontuna.net`,
  `ssh.dongwontuna.net`, `nvidia-lb.dongwontuna.net`, and `orca.dongwontuna.net` need the
  WARP DNS proxy to observe resolution and pin the resulting addresses into tunnel routes.
  Traffic only mode has no DNS proxy, so the dashboard disables those rows outright. This is
  safe today because the host reaches codex-lb at `http://127.0.0.1:2455/backend-api/codex`
  and does not dial those names. Anything added later that calls them from this host will
  egress outside the tunnel and may fail Cloudflare Access policies that require WARP.
- **Gateway DNS filtering and logging no longer cover this host.** Queries go to the LAN
  resolvers. Restoring visibility means a resolver policy, not a return to WARP DNS.
- IP-based include entries and the `192.168.1.148/32` route are unaffected.
- **The resolver of last resort is weaker than WARP DoH was.** WARP DoH used to resolve
  regardless of what the local network offered; now the host follows whatever DHCP hands out.
  Both LAN resolvers answered 3/3 in every probe, and this is a fixed-location server, so the
  exposure is the narrow case where DHCP supplies no resolver or a dead one.
  `/etc/systemd/resolved.conf.d/20-fallback-dns.conf` now sets
  `FallbackDNS=8.8.8.8 8.8.4.4 9.9.9.9` to cover the first half of that. It does **not** cover
  the second half: resolved engages FallbackDNS only when no server is configured at all, so a
  configured-but-dead DHCP resolver still has no automatic escape.

Measured on this uplink on 2026-08-26, three probes per server, `chatgpt.com` A record:

| Resolver | UDP 53 | TCP 53 | DoT 853 |
| --- | --- | --- | --- |
| `172.20.11.10` (LAN) | 3/3 | 3/3 | not offered |
| `8.8.8.8` (Google) | 1/3 | 3/3 | 3/3, 0.03s |
| `1.1.1.1` (Cloudflare) | 2/3 | 1/3 | 3/3, 0.06-0.08s |
| `9.9.9.9` (Quad9) | 1/3 | 3/3 | 3/3, 0.03-1.06s |

External UDP 53 is unreliable here for every provider, not blocked per vendor — a single
probe is not enough to judge it. TCP 53 and DoT are solid, and Google is the fastest and
most consistent of the three on both. External IPv6 resolvers fail immediately with
`OSError`; this host has no external IPv6 egress. Any fallback list should therefore be
IPv4 and Google-first. Enabling `DNSOverTLS=opportunistic` would buy the most reliable
transport but is global, so it would also make every lookup probe port 853 against the LAN
resolvers that do not offer it — a new failure surface on the one path that currently
measures perfect.

## Alternatives

1. **Scope the WARP resolved link to `~dongwontuna.net` and drop `~.`.** Would have kept
   Gateway DNS for those names. Rejected because warp-svc rewrites its resolved
   configuration on every reconnect and route change, so it needs a timer or watcher unit
   fighting the daemon indefinitely — and while the DoH upstream is down those names would
   resolve nowhere anyway.
2. **Change the `Default` profile service mode.** Rejected: all four enrolled devices share
   it, so the Mac would lose Gateway DNS filtering and its host-based split tunnel entries.
3. **Pin `dns:` on the codex-lb compose service.** Applied first as immediate relief and then
   reverted once host DNS was fixed. It hardcodes DHCP-supplied resolver addresses and only
   rescues one container while the rest of the host stays broken.
4. **Wait for the upstream path to recover.** Rejected as a standing posture: it leaves the
   original single point of failure in place for the next outage.

## Rollback

Set the profile service mode back to Traffic and DNS mode, or disable or delete the profile
so the host falls through to `Default`. The client picks the change up within about a minute;
confirm with `warp-cli settings | grep Mode:` and `resolvectl domain | grep -i cloudflare`.
