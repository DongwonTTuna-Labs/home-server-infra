#!/usr/bin/env bash
set -euo pipefail

display="${DISPLAY:-:99}"
screen="${GWP_SCREEN:-1366x900x24}"
chrome="${CHROME_BINARY_PATH:-/usr/bin/chromium}"
base_url="${GWP_BASE_URL:-https://chatgpt.com}"
runtime_dir="/tmp/gwp-runtime"

if [[ ! "${GWP_DAEMON_PORT:-}" =~ ^[1-9][0-9]{0,4}$ ]] || (( GWP_DAEMON_PORT > 65535 )); then
  echo "GWP_DAEMON_PORT must be an integer between 1 and 65535" >&2
  exit 1
fi
if [[ ! "${GWP_DAEMON_TOKEN:-}" =~ ^[0-9a-f]{32}$ ]]; then
  echo "GWP_DAEMON_TOKEN must be exactly 32 lower-hex characters" >&2
  exit 1
fi

mkdir -p /profile /outbox "$runtime_dir"
chmod 0700 "$runtime_dir"
rm -f /profile/SingletonLock /profile/SingletonCookie /profile/SingletonSocket

display_number="${display#:}"
rm -f "/tmp/.X${display_number}-lock" "/tmp/.X11-unix/X${display_number}"
Xvfb "$display" -screen 0 "$screen" -nolisten tcp -ac >"$runtime_dir/xvfb.log" 2>&1 &
echo "$!" >"$runtime_dir/xvfb.pid"

for _ in $(seq 1 80); do
  [[ -S "/tmp/.X11-unix/X${display_number}" ]] && break
  sleep 0.25
done

DISPLAY="$display" XDG_RUNTIME_DIR="$runtime_dir" "$chrome" \
  --no-sandbox \
  --disable-setuid-sandbox \
  --disable-dev-shm-usage \
  --disable-gpu \
  --disable-component-update \
  --disable-background-networking \
  --disable-sync \
  --remote-debugging-address=127.0.0.1 \
  --remote-debugging-port=9222 \
  --user-data-dir=/profile \
  "$base_url" >"$runtime_dir/chromium.log" 2>&1 &
echo "$!" >"$runtime_dir/chromium.pid"

cdp_ready=0
for _ in $(seq 1 120); do
  if curl -fsS http://127.0.0.1:9222/json/version >/dev/null 2>&1; then
    cdp_ready=1
    break
  fi
  sleep 0.25
done
if [[ "$cdp_ready" != "1" ]]; then
  echo "Chromium CDP did not become ready" >&2
  exit 1
fi

exec node /app/dist/daemon/main.js
