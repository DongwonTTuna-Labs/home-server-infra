# Orca Home

`orca-home` runs Orca's headless runtime on the home server and advertises the
public WebSocket endpoint `wss://orca.dongwontuna.net`. Cloudflare Named Tunnel
terminates TLS and forwards WebSocket upgrades to the local Orca origin on port
`6768`.

This deployment does not use Orca Relay. Relay is useful when a direct public
endpoint cannot be operated, but the existing `tunnel-apps` connector already
provides a stable authenticated path without adding another relay dependency.

## Boundary and private state

The pinned Orca CLI does not expose a bind-address option. `orca serve` listens
on `0.0.0.0:6768`, so the origin may also be reachable directly from the LAN
when the host firewall permits it. Cloudflare is the supported remote entrypoint;
do not forward port `6768` on the router.

`--json` prints runtime pairing authorization data. The service's small
`orca-home-run` wrapper therefore creates the private state boundary and
redirects standard output to
`${XDG_STATE_HOME:-$HOME/.local/state}/orca-home/serve-ready.json`; the
systemd standard-output target itself is `/dev/null`, never the journal. The
state directory is mode `0700`, the file is mode `0600`, and the file must be
treated as a secret. Standard error remains available in the user journal for
diagnostics. A healthy file is exactly Orca's versioned, single-line
`orca_server_ready` JSON contract with `pairing.scope` set to `runtime`.
The installer reads `XDG_STATE_HOME` from the user manager, matching the
environment inherited by the service rather than assuming the invoking shell
has the same value.

Do not add `--mobile-pairing` to this service. Upstream uses that switch to
mint a `scope=mobile` offer with restricted RPC permissions; it cannot serve as
the saved desktop remote environment named `home`. With the switch omitted,
`orca serve` issues the required `scope=runtime` pairing offer.

Orca's [official headless Linux guide](https://github.com/stablyai/orca/blob/v1.4.156/docs/reference/headless-linux-server.md)
warns that `--appimage-extract-and-run` can print extracted paths before the
ready JSON. The installer follows the documented automation path instead: it
verifies the AppImage, runs `--appimage-extract` once, and the service invokes
the versioned `squashfs-root/AppRun` directly. This avoids both FUSE and stdout
filtering. The extracted launcher also receives an explicit `APPDIR`, matching
Orca's headless Docker runner; Orca's packaged CLI redirect requires `APPIMAGE`
or `APPDIR` in the inherited environment before it recognizes the `serve`
subcommand.

This host sets `kernel.apparmor_restrict_unprivileged_userns=1`, and
`unshare -Ur true` fails for the service user. Orca's packaged `AppRun` uses
that exact probe and falls back to `--no-sandbox`; the CLI then propagates the
operator choice to the Electron child through `ORCA_APPIMAGE_NO_SANDBOX`.
The unit declares `--no-sandbox` explicitly, before `serve`, rather than
depending on a hidden launcher heuristic. This disables Chromium's process
sandbox, so it is an intentional host-specific exception. Keep the surrounding
systemd controls (`NoNewPrivileges`, read-only release files, private temporary
storage, restrictive umask, and private readiness output) in place, and do not
remove the flag unless a separately validated AppArmor/user-namespace or
setuid-sandbox deployment replaces this exception.

## Bootstrap pin and latest channel

[`release.json`](release.json) pins the official `stablyai/orca` Linux
AppImage by version, byte size, GitHub-published SHA-256, and a deterministic
SHA-256 of the complete extracted tree. The installer verifies that tree both
after extraction and before reusing an existing release, records the release
source commit, verifies the x86-64 ELF asset, and installs the release under
`~/.local/orca/releases/v1.4.156/`. The stable `~/.local/orca/current` symlink
selects the active release for systemd. The pinned release is the deterministic
bootstrap and recovery floor; it is not an upper bound on the active version.

`orca-update-latest.timer` checks GitHub's latest non-draft, non-prerelease
release hourly with up to ten minutes of jitter. The updater requires both the
GitHub asset SHA-256 and the upstream `latest-linux.yml` SHA-512, verifies the
x86-64 AppImage and deterministic extracted tree, and publishes the new release
with `mv -T`. It never follows an unversioned asset URL after discovery and
never downgrades the current semantic version.

Orca persists state under both `~/.config/orca/` and `~/.config/Orca/`.
Immediately before switching versions, the updater stops only
`orca-serve.service`. If Orca left its version-matched persistent daemon in an
`app-orca-*.scope`, the updater accepts exactly one live, user-owned,
mode-`0600` daemon record and verifies its PID, release entry path, app version,
process name, and exact scope before sending that PID `SIGTERM`; unrelated
processes and ambiguous records fail closed.

The updater then writes a mode-`0600` profile snapshot below the mode-`0700`
Orca state directory. An `.incomplete` marker covers snapshot creation, and
the completed manifest records the archive size and SHA-256. It then requires
the private readiness contract, canonical project/worktree, and local plus
public WebSocket `101`. Any failure restores the previous symlink and profile
snapshot, restarts the old runtime, and blocks the failed tag until a newer
release appears. An explicitly stopped `orca-serve.service` is preserved
rather than restarted by the updater.

Retention always keeps the active release, its immediately previous release,
and every non-dynamic bootstrap pin. It keeps the one complete rollback bundle
referenced by the current successful update state. Older directories are
removed only when they satisfy the updater's exact private ownership, naming,
manifest, archive, and full dynamic-release integrity contracts; unknown,
corrupt, symlinked, or user-created paths are preserved. A later updater run
also removes an interrupted snapshot only when its trusted `.incomplete`
marker and restricted partial contents validate.

## Install or refresh

Run from the canonical checkout:

```bash
stacks/orca-home/scripts/install.sh --activate
```

To install an already-downloaded, checksum-matching asset without downloading
it again:

```bash
stacks/orca-home/scripts/install.sh \
  --source "$HOME/.local/orca/orca-linux.AppImage.new" \
  --activate
```

The installer also installs and enables `orca-serve.service` and the hourly
latest-channel timer. The runtime unit uses `Restart=always`, so both clean and
failed process exits recover after five seconds; an explicit
`systemctl --user stop orca-serve.service` still remains stopped. `Xvfb`, `jq`,
`file`, `flock`, `git`, `node`, `tar`, and the normal Electron runtime
libraries must already be present. A mode-`0600` host-local install lock is
shared by the installer and updater, serializing release verification,
extraction, unit installation, activation, and bootstrap. Staged releases use
`mv -T`, so an unexpected concurrent destination fails instead of nesting a
second extracted tree under a release.

After runtime readiness, the installer uses the active version-matched Orca CLI
and the private runtime pairing only in-process to idempotently register
`$HOME/Documents/Programming/home-server-infra` as the initial server-owned
project. It then requires the matching runtime worktree before declaring
activation successful. The pairing value is never passed as an argument or
printed. The service sets `LIBGL_ALWAYS_SOFTWARE=1` plus the extracted `APPDIR`,
clears any inherited `DISPLAY`, passes the required `--no-sandbox` before
`serve`, and lets Orca start its documented private Xvfb instance. User linger
must remain enabled so Orca returns after reboot:

```bash
loginctl show-user "$USER" -p Linger
```

## Select `home` as the desktop Active Server

Saving the pairing as `home` and clicking **Connect** proves reachability, but
does not re-home an existing local-owned workspace. Orca v1.4.156 deliberately
keeps connection state separate from its durable Active Server preference. On
the desktop client, select:

`Settings` → `Remote Orca Servers` → `Advanced` → `Active Server` → `home`

Then open the `home-server-infra` project published by `home`. The installer
registers that project on the runtime because an empty server catalog contains
no server-owned workspace for the desktop to select. If the project list was
already open during bootstrap, reconnect `home` once to refresh it.

An existing local-owned workspace continues to use the local daemon even after
the server is connected. If that daemon receives a server-only path such as
`/home/dongwonttuna`, it reports `DaemonProtocolError: Working directory ...
does not exist` because it is validating the path on the desktop machine. That
message does not mean the server home directory disappeared. Do not reopen the
stale local workspace; open the `home`-owned `home-server-infra` entry instead.
The server can publish and own the project, but it cannot rewrite a local
workspace record already saved on the desktop.

For CLI calls, select the saved runtime explicitly with `--environment home`,
or set `ORCA_ENVIRONMENT=home` for the CLI session. Pairing codes and URLs
remain secret in either flow.

## Validate and publish

Validate the tracked files first:

```bash
bash -n stacks/orca-home/scripts/*.sh
systemd-analyze --user verify \
  stacks/orca-home/systemd/orca-serve.service \
  stacks/orca-home/systemd/orca-update-latest.service \
  stacks/orca-home/systemd/orca-update-latest.timer
cloudflared tunnel \
  --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml \
  ingress validate
cloudflared tunnel \
  --config stacks/tunnel-apps/cloudflared/tunnel-apps.yml \
  ingress rule https://orca.dongwontuna.net/
docker compose -f stacks/tunnel-apps/compose.yaml config --quiet
```

Check the private local state without printing its contents:

```bash
set -euo pipefail

service_xdg_state_home=$(
  systemctl --user show-environment \
    | sed -n 's/^XDG_STATE_HOME=//p'
)
state_root=${service_xdg_state_home:-$HOME/.local/state}
test "${state_root#/}" != "$state_root"
readiness=$state_root/orca-home/serve-ready.json
systemctl --user is-enabled orca-serve.service
systemctl --user is-active orca-serve.service
systemctl --user is-enabled orca-update-latest.timer
systemctl --user is-active orca-update-latest.timer
test "$(systemctl --user show orca-serve.service -p Restart --value)" = always
systemctl --user show orca-serve.service -p ExecStart --value \
  | grep -F -- 'AppRun --no-sandbox serve'
test "$(stat -c '%a' "$readiness")" = 600
jq -e '
  .type == "orca_server_ready" and
  .schemaVersion == 1 and
  .boundEndpoint == "ws://0.0.0.0:6768" and
  .advertisedEndpoint == "wss://orca.dongwontuna.net" and
  .pairing.available == true and
  .pairing.endpoint == "wss://orca.dongwontuna.net" and
  .pairing.scope == "runtime"
' "$readiness" >/dev/null
ss -ltn 'sport = :6768' | grep -F ':6768'

(
  export ORCA_PAIRING_CODE
  ORCA_PAIRING_CODE=$(jq -er '.pairing.url' "$readiness")
  orca_cli=$HOME/.local/orca/current/squashfs-root/resources/app.asar.unpacked/out/cli/index.js
  node "$orca_cli" repo list --json \
    | jq -e --arg path "$HOME/Documents/Programming/home-server-infra" '
        .ok == true and
        any(.result.repos[]; .path == $path and .kind == "git")
      ' >/dev/null
)

$HOME/.local/libexec/orca-home-update-latest --check
```

Recreate only the shared application-tunnel connector, then confirm its active
connections before creating or replacing the DNS route:

```bash
set -euo pipefail

docker compose -f stacks/tunnel-apps/compose.yaml \
  up -d --force-recreate cloudflared-apps
cloudflared tunnel info tunnel-apps
docker logs cloudflared-apps 2>&1 | grep 'Registered tunnel connection'
cloudflared tunnel route dns --overwrite-dns \
  tunnel-apps orca.dongwontuna.net
```

Verify the public WebSocket upgrade without using or printing the private
pairing URL:

```bash
set -euo pipefail

headers=$(mktemp)
trap 'rm -f -- "$headers"' EXIT
curl --http1.1 --silent --show-error --max-time 3 \
  --dump-header "$headers" --output /dev/null \
  --header 'Connection: Upgrade' \
  --header 'Upgrade: websocket' \
  --header 'Sec-WebSocket-Version: 13' \
  --header 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  https://orca.dongwontuna.net/ || test "$?" = 28
grep -Eq '^HTTP/1[.]1 101 ' "$headers"
```

## Rollback and withdrawal

Do not roll back only `current` or only the AppImage. Automatic update failures
restore one complete profile-plus-binary generation. The last update outcome is
recorded without pairing material in
`${XDG_STATE_HOME:-$HOME/.local/state}/orca-home/update-state.json`; rollback
archives in the adjacent `update-rollbacks/` directory may contain credentials
and must remain mode `0600` inside its mode-`0700` parent. Only the current
successful generation is retained automatically; unrecognized or failed
forensic directories are deliberately left for an operator. A failed tag is
recorded in `update-blocked.json` so the hourly timer does not repeatedly cause
an outage. Retry it only after diagnosing the journal and explicitly running
`orca-home-update-latest --apply --force`.

For a full withdrawal, remove the `orca.dongwontuna.net` DNS record in
Cloudflare, remove its ingress rule, recreate only `cloudflared-apps`, and then
disable `orca-serve.service`. Do not stop the separate `cloudflared` container;
it carries SSH traffic.
