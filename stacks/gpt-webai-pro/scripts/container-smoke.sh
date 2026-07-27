#!/usr/bin/env bash
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
smoke_dir="$(mktemp -d)"
container="gwp-v2-smoke-$$"
harness_pid=""
daemon_port="19399"
daemon_token="$(node -e 'process.stdout.write(require("node:crypto").randomBytes(16).toString("hex"))')"
token_path="$smoke_dir/daemon.token"

cleanup() {
  docker rm -f "$container" >/dev/null 2>&1 || true
  if [[ -n "$harness_pid" ]]; then kill "$harness_pid" >/dev/null 2>&1 || true; fi
  rm -rf -- "$smoke_dir"
}
trap cleanup EXIT

mkdir -p "$smoke_dir/profile" "$smoke_dir/outbox"
umask 077
printf '%s\n' "$daemon_token" >"$token_path"
chmod 0600 "$token_path"
node --import tsx "$root/test/fake-chatgpt/server.ts" --port 18765 >"$smoke_dir/harness.log" 2>&1 &
harness_pid="$!"
for _ in $(seq 1 80); do
  curl -fsS http://127.0.0.1:18765/ >/dev/null 2>&1 && break
  sleep 0.25
done
curl -fsS http://127.0.0.1:18765/ >/dev/null

docker build -f "$root/container/Dockerfile" -t home-server/gpt-webai-pro-slot:smoke "$root"
docker run -d --name "$container" --network host \
  --user "$(id -u):$(id -g)" \
  --mount "type=bind,src=$smoke_dir/profile,dst=/profile" \
  --mount "type=bind,src=$smoke_dir/outbox,dst=/outbox" \
  -e GWP_BASE_URL=http://127.0.0.1:18765/?scenario=happy \
  -e GWP_DAEMON_PORT="$daemon_port" \
  -e GWP_DAEMON_TOKEN="$daemon_token" \
  home-server/gpt-webai-pro-slot:smoke >/dev/null

cd "$root"
DAEMON_PORT="$daemon_port" DAEMON_TOKEN_PATH="$token_path" node --input-type=module <<'NODE'
import { RpcClient } from './dist/supervisor/rpc-client.js';
import { createHash } from 'node:crypto';
const port = Number(process.env.DAEMON_PORT);
const deadline = Date.now() + 60000;
let rpc = null;
while (Date.now() < deadline) {
  try {
    rpc = await RpcClient.connect(port, process.env.DAEMON_TOKEN_PATH, 1000);
    const health = await rpc.call('health', undefined, 1000);
    if (health.ok === true && health.chromeConnected === true) break;
  } catch {
    // Chromium and the daemon may still be starting.
  }
  if (rpc) await rpc.close().catch(() => undefined);
  rpc = null;
  await new Promise((resolve) => setTimeout(resolve, 250));
}
if (!rpc) throw new Error(`daemon did not become healthy on 127.0.0.1:${port}`);
const ready = await rpc.call('readiness', undefined);
if (ready.state !== 'ready') throw new Error(JSON.stringify(ready));
const prompt = 'container smoke';
const sent = await rpc.call('send', { prompt, files: [], newConversation: true });
const polled = await rpc.call('poll', {
  conversationUrl: sent.conversationUrl,
  promptSha256: createHash('sha256').update(prompt).digest('hex'),
  userTurnId: sent.userTurnId,
  assistantTurnId: sent.assistantTurnId,
  waitMs: 10000,
});
if (polled.state !== 'complete') throw new Error(JSON.stringify(polled));
await rpc.call('closeConversation', { conversationUrl: polled.currentUrl });
await rpc.close();
process.stdout.write(`${JSON.stringify({ ready, sent, polled })}\n`);
NODE
