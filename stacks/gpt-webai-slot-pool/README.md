# gpt-webai-slot-pool

Docker-backed ChatGPT/GPT Pro slot pool for `gpt-webai-lifecycle`.

This stack exists because the upstream `agbrowse --parallel` tab pool has not
been stable enough for long-running Pro delegation. The lifecycle supervisor
owns allocation, session ledgers, attachment staging, duplicate-send prevention,
and slot recovery. The Docker stack only provides isolated browser runtimes.

## Architecture

- Ten explicit services: `gpt-webai-slot-01` through `gpt-webai-slot-10`
- One Chromium + Xvfb runtime per slot
- One persistent Chrome profile per slot:
  `/state/slot-N/browser-profile`
- One `BROWSER_AGENT_HOME` per slot:
  `/state/slot-N`
- One CDP port per slot inside the container:
  `9223` through `9232`
- One read-only attachment mount per slot:
  `/broker-attachments`
- No host-published ports in normal operation
- Access from automation is only through `gpt-webai-lifecycle` and
  `docker exec`

The normal entrypoints are:

```bash
gptpro "prompt"
gptxhigh "prompt"
gptpro --file /path/to/context.zip "prompt"
gpt-webai-lifecycle status
gpt-webai-lifecycle browser ensure --slot slot-01
gpt-webai-lifecycle resume --kind pro --session SESSION_ID
gpt-webai-lifecycle queue resume --request REQUEST_FINGERPRINT
```

Do not use raw `agbrowse web-ai` or MCP `web_ai_*` tools for ordinary GPT
delegation. Those paths bypass the slot ledger and duplicate-send guardrails.

## Included Supervisor

This stack also vendors the lifecycle supervisor used by the slot broker:

```text
stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle
```

Install or refresh the operator copy with:

```bash
install -m 0755 \
  stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle \
  "$HOME/.local/bin/gpt-webai-lifecycle"
```

The supervisor owns:

- ready-slot allocation and queued envelopes
- `sessionId -> slotId` pinning for resume/poll
- duplicate-send prevention by request fingerprint
- slot-specific `docker exec` execution
- attachment capsule staging into `/broker-attachments`
- compound extension preservation for files such as `.tar.gz`
- `ATTACHMENT_ACCESS_GATE` prompts and `provider.attachment_unavailable`
  recovery envelopes
- ChatGPT login-state gating; a visible login/signup UI makes the slot
  `auth.needs_login`/`reseed_login`, not `ready`

Offline regression tests live under:

```text
stacks/gpt-webai-slot-pool/tests/gpt-webai-lifecycle
```

The repo copy of the operator runbook is:

```text
stacks/gpt-webai-slot-pool/docs/gpt-webai-lifecycle-runbook.md
```

If the active Codex runbook needs to be refreshed from this repo, copy it to
`$HOME/.codex/runbooks/gpt-webai-lifecycle.md`.

## Bootstrap

Let the lifecycle supervisor create the host state directories with the same
UID/GID that the containers will run as:

```bash
STATE="${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-lifecycle"

GPT_WEBAI_SLOT_MODE=on GPT_WEBAI_SLOT_COUNT=10 gpt-webai-lifecycle status

find "$STATE/slots" -maxdepth 2 \( -name state -o -name attachments \) -type d -print

export GPT_WEBAI_SLOT_UID="$(id -u)"
export GPT_WEBAI_SLOT_GID="$(id -g)"
export GPT_WEBAI_STATE_DIR="$STATE"
```

Validate and start the stack:

```bash
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml config
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml up -d --build
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml ps
```

Container health only proves Chromium/CDP is reachable. It does not prove the
slot is logged into ChatGPT or can use Pro.

## Manual ChatGPT Login

Each slot has an independent Chrome profile. In practice, copying an existing
host profile or seed profile may still leave ChatGPT logged out because browser
cookies/session state can be profile, device, version, or provider bound. Treat
per-slot manual login as the reliable setup path.

The stack intentionally publishes no browser or CDP ports. For login, open a
temporary, operator-controlled CDP bridge for one slot at a time, use an SSH
tunnel, finish login, then stop the bridge. Do not expose these ports publicly.

On the server, choose the slot and create a temporary in-container bridge:

```bash
slot=01
container="gpt-webai-slot-$slot"
slot_name="slot-$slot"
cdp_port="$((9222 + 10#$slot))"
bridge_port="$((19000 + 10#$slot))"

docker exec -d \
  --env BRIDGE_PORT="$bridge_port" \
  --env CDP_PORT="$cdp_port" \
  "$container" sh -lc "
  mkdir -p /state/$slot_name/run
  node -e '
    const net = require(\"node:net\");
    const listen = Number(process.env.BRIDGE_PORT);
    const target = Number(process.env.CDP_PORT);
    const server = net.createServer((client) => {
      const upstream = net.connect(target, \"127.0.0.1\");
      client.pipe(upstream);
      upstream.pipe(client);
      const close = () => { client.destroy(); upstream.destroy(); };
      client.on(\"error\", close);
      upstream.on(\"error\", close);
    });
    server.listen(listen, \"0.0.0.0\");
    setInterval(() => {}, 60000);
  ' >/state/$slot_name/run/login-cdp-bridge.log 2>&1 &
  echo \$! >/state/$slot_name/run/login-cdp-bridge.pid
"

container_ip="$(docker inspect -f '{{range.NetworkSettings.Networks}}{{.IPAddress}}{{end}}' "$container")"
printf 'slot=%s container_ip=%s bridge_port=%s\n' "$slot_name" "$container_ip" "$bridge_port"
```

From your workstation, create an SSH tunnel to the server. Replace `SERVER` with
the host you normally SSH into:

```bash
ssh -N -L 19001:CONTAINER_IP:19001 SERVER
```

Open this URL locally:

```text
http://127.0.0.1:19001/json/list
```

Open the listed `devtoolsFrontendUrl` relative to
`http://127.0.0.1:19001`, use the DevTools screencast to interact with the
ChatGPT page, and complete login for that slot. Repeat for slots `01` through
`10`, changing the local/bridge port to `19002`, `19003`, and so on.

After each slot login, stop the temporary bridge:

```bash
slot=01
container="gpt-webai-slot-$slot"
slot_name="slot-$slot"

docker exec "$container" sh -lc '
  pid_file="/state/'"$slot_name"'/run/login-cdp-bridge.pid"
  if [ -s "$pid_file" ]; then
    kill "$(cat "$pid_file")" 2>/dev/null || true
    rm -f "$pid_file"
  fi
'
```

## Login Verification

Verify every slot through the lifecycle supervisor, not through Docker health:

```bash
for i in $(seq -w 1 10); do
  printf 'slot-%s ' "$i"
  gpt-webai-lifecycle browser ensure --slot "slot-$i"
done

gpt-webai-lifecycle status
```

Expected healthy state:

```text
slot_01_status=ready
...
slot_10_status=ready
```

If a slot shows `auth.needs_login` or `reseed_login`, it is not an authenticated
Pro slot. Do not trust responses, attachments, or ChatGPT sidebar history from
that slot until login verification passes.

## Attachment Handling

`gpt-webai-lifecycle` never passes original host paths directly into the
container. It creates a host-only attachment capsule and exposes generated
read-only filenames to the chosen slot:

```text
/broker-attachments/<request>/<run>/files/NNN-<sha256-16><safeExt>
```

Directory evidence should be zipped or tarred before attaching:

```bash
python3 -m zipfile -c /path/to/context.zip /path/to/context-dir
gptpro --file /path/to/context.zip "review this evidence"
```

For attachment requests, the lifecycle prompt includes an
`ATTACHMENT_ACCESS_GATE`. If the provider replies `ATTACHMENT_MISSING` or the
wrapper returns `reason:"provider.attachment_unavailable"`, do not treat the
result as a file-based review success.

## Recovery Semantics

- A `sessionId` maps to the original `slotId`; resumes stay on that slot.
- If all slots are busy or repairing, the supervisor returns a queued envelope
  with `queue resume --request REQUEST_FINGERPRINT`.
- If a send starts but no `sessionId` is recorded, do not resend the same
  fingerprint. Follow the `send.unknown_session` recovery envelope.
- If a slot is `repairing`, `warming`, `reseed_login`, or `degraded`, the broker
  will not allocate it for new work.
- Login state is part of readiness. Composer visibility alone is not enough.

## Security Notes

- No CDP ports are published in compose.
- Temporary login bridges are operator-only and must be stopped after login.
- Do not commit, print, or attach Chrome profile files, cookies, tokens, or
  `$STATE/auth-seed/**`.
- Do not delete user Chrome profiles during cleanup.
- Slot attachment mounts are read-only and contain generated filenames, not
  original host paths.

## Smoke Checks

```bash
bash -n stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle
bash -n stacks/gpt-webai-slot-pool/scripts/slot-entrypoint.sh
bash -n stacks/gpt-webai-slot-pool/scripts/slot-healthcheck.sh
docker compose -f stacks/gpt-webai-slot-pool/compose.yaml config --services
GPT_WEBAI_TEST_ROOT="$PWD/.omo/evidence/gpt-webai-lifecycle/local" \
  stacks/gpt-webai-slot-pool/tests/gpt-webai-lifecycle/test.sh all
```

For a real end-to-end smoke, first complete manual login for at least one slot,
then run:

```bash
gpt-webai-lifecycle browser ensure --slot slot-01
gptpro "Reply with exactly: GPT_SLOT_OK"
```
