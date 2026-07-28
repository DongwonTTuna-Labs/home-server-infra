#!/usr/bin/env bash
set -euo pipefail

display="${DISPLAY:-:99}"
login_mode="${GWP_LOGIN_MODE:-0}"
if [[ "$login_mode" == "1" ]]; then
  screen="${GWP_SCREEN:-1440x900x24}"
else
  screen="${GWP_SCREEN:-1366x900x24}"
fi
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
if [[ "$login_mode" != "0" && "$login_mode" != "1" ]]; then
  echo "GWP_LOGIN_MODE must be 0 or 1" >&2
  exit 1
fi
if [[ "$login_mode" == "1" ]]; then
  if [[ ! "${GWP_NOVNC_PORT:-}" =~ ^[1-9][0-9]{0,4}$ ]] || (( GWP_NOVNC_PORT > 65535 )); then
    echo "GWP_NOVNC_PORT must be an integer between 1 and 65535 in login mode" >&2
    exit 1
  fi
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

if [[ "$login_mode" == "1" ]]; then
  DISPLAY="$display" x11vnc \
    -display "$display" \
    -localhost \
    -forever \
    -shared \
    -nopw \
    -rfbport 5900 >"$runtime_dir/x11vnc.log" 2>&1 &
  echo "$!" >"$runtime_dir/x11vnc.pid"

  websockify \
    --web=/usr/share/novnc \
    "0.0.0.0:${GWP_NOVNC_PORT}" \
    127.0.0.1:5900 >"$runtime_dir/websockify.log" 2>&1 &
  echo "$!" >"$runtime_dir/websockify.pid"
fi

DISPLAY="$display" XDG_RUNTIME_DIR="$runtime_dir" "$chrome" \
  --no-sandbox \
  --disable-setuid-sandbox \
  --disable-dev-shm-usage \
  --disable-gpu \
  --disable-component-update \
  --disable-background-networking \
  --disable-background-timer-throttling \
  --disable-backgrounding-occluded-windows \
  --disable-renderer-backgrounding \
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

# exec 금지: SIGTERM을 Chrome까지 전달해 프로필(쿠키 DB) flush를 보장해야 한다.
# Chromium은 주기(~30s) flush라 하드킬되면 직전 로그인 쿠키가 유실된다 (2026-07-28 실측).
node /app/dist/daemon/main.js &
daemon_pid="$!"

shutdown() {
  # 순서가 생명이다: daemon이 CDP Browser.close로 Chrome을 클린 종료(쿠키 flush)할 때까지
  # 기다린 뒤에만 fallback TERM을 보낸다. Chrome에 먼저/동시에 SIGTERM을 보내면
  # Chromium이 비플러시 경로로 종료해 직전 로그인/회전 쿠키가 유실된다 (실측).
  kill -TERM "$daemon_pid" 2>/dev/null || true
  for _ in $(seq 1 125); do
    kill -0 "$daemon_pid" 2>/dev/null || break
    sleep 0.2
  done
  chrome_pid="$(cat "$runtime_dir/chromium.pid" 2>/dev/null || true)"
  if [[ -n "$chrome_pid" ]] && kill -0 "$chrome_pid" 2>/dev/null; then
    kill -TERM "$chrome_pid" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "$chrome_pid" 2>/dev/null || break
      sleep 0.2
    done
  fi
  exit 0
}
trap shutdown TERM INT

wait "$daemon_pid"
