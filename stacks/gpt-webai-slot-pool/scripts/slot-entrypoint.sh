#!/usr/bin/env bash
set -euo pipefail

: "${BROWSER_AGENT_HOME:?BROWSER_AGENT_HOME is required}"
: "${CDP_PORT:?CDP_PORT is required}"

display="${DISPLAY:-:99}"
screen="${GPT_WEBAI_SLOT_SCREEN:-1366x900x24}"
profile="$BROWSER_AGENT_HOME/browser-profile"
run_dir="$BROWSER_AGENT_HOME/run"
chrome="${CHROME_BINARY_PATH:-/usr/bin/chromium}"

mkdir -p "$profile" "$run_dir"

if ! pgrep -f "Xvfb $display " >/dev/null 2>&1; then
  Xvfb "$display" -screen 0 "$screen" -nolisten tcp -ac >"$run_dir/xvfb.log" 2>&1 &
  echo "$!" > "$run_dir/xvfb.pid"
fi

for _ in $(seq 1 40); do
  [[ -S "/tmp/.X11-unix/X${display#:}" ]] && break
  sleep 0.25
done

rm -f "$profile"/SingletonLock "$profile"/SingletonCookie "$profile"/SingletonSocket

if ! curl -fsS "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null 2>&1; then
  xdg_runtime_dir="$BROWSER_AGENT_HOME/xdg-runtime"
  mkdir -p "$xdg_runtime_dir"
  chmod 700 "$xdg_runtime_dir"

  DISPLAY="$display" XDG_RUNTIME_DIR="$xdg_runtime_dir" "$chrome" \
    --no-sandbox \
    --disable-setuid-sandbox \
    --disable-seccomp-filter-sandbox \
    --disable-dev-shm-usage \
    --disable-gpu \
    --disable-software-rasterizer \
    --disable-component-update \
    --disable-background-networking \
    --disable-sync \
    --disable-features=UseOzonePlatform,VizDisplayCompositor,MediaRouter,OptimizationHints,AutofillServerCommunication \
    --remote-debugging-address=127.0.0.1 \
    --remote-debugging-port="$CDP_PORT" \
    --user-data-dir="$profile" \
    "https://chatgpt.com/" >"$run_dir/chrome.log" 2>&1 &
  echo "$!" > "$run_dir/chrome.pid"
fi

cdp_ready=0
for _ in $(seq 1 80); do
  if curl -fsS "http://127.0.0.1:$CDP_PORT/json/version" >/dev/null 2>&1; then
    cdp_ready=1
    break
  fi
  sleep 0.25
done

if [[ "$cdp_ready" != "1" ]]; then
  echo "CDP did not become ready on port $CDP_PORT" >&2
  exit 1
fi

wait -n
