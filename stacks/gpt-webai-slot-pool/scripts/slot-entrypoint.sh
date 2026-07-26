#!/usr/bin/env bash
set -euo pipefail

: "${BROWSER_AGENT_HOME:?BROWSER_AGENT_HOME is required}"
: "${CDP_PORT:?CDP_PORT is required}"
: "${PR72_OWNER_ID:?PR72_OWNER_ID is required}"
: "${PR72_OWNER_GENERATION:?PR72_OWNER_GENERATION is required}"
: "${PR72_RUNTIME_INCARNATION:?PR72_RUNTIME_INCARNATION is required}"

[[ "$PR72_OWNER_ID" =~ ^owner_[0-9a-f]{64}$ ]] || {
  echo "invalid PR72_OWNER_ID" >&2
  exit 1
}
[[ "$PR72_OWNER_GENERATION" =~ ^[0-9]+$ ]] \
  && (( PR72_OWNER_GENERATION >= 1 && PR72_OWNER_GENERATION <= 65535 )) || {
  echo "invalid PR72_OWNER_GENERATION" >&2
  exit 1
}
[[ "$PR72_RUNTIME_INCARNATION" =~ ^runtime_[0-9a-f]{64}$ ]] || {
  echo "invalid PR72_RUNTIME_INCARNATION" >&2
  exit 1
}

display="${DISPLAY:-:99}"
screen="${GPT_WEBAI_SLOT_SCREEN:-1366x900x24}"
profile="$BROWSER_AGENT_HOME/browser-profile"
run_dir="$BROWSER_AGENT_HOME/run"
chrome="${CHROME_BINARY_PATH:-/usr/bin/chromium}"
display_num="${display#:}"
x11_dir="${GPT_WEBAI_X11_DIR:-/tmp/.X11-unix}"
x11_lock="${GPT_WEBAI_X11_LOCK:-/tmp/.X${display_num}-lock}"
x11_socket="$x11_dir/X${display_num}"

mkdir -p "$profile" "$run_dir"

if ! pgrep -f "Xvfb $display " >/dev/null 2>&1; then
  rm -f "$x11_lock" "$x11_socket"
  Xvfb "$display" -screen 0 "$screen" -nolisten tcp -ac >"$run_dir/xvfb.log" 2>&1 &
  echo "$!" > "$run_dir/xvfb.pid"
fi

for _ in $(seq 1 40); do
  [[ -S "$x11_socket" ]] && break
  sleep 0.25
done

rm -f "$profile"/SingletonLock "$profile"/SingletonCookie "$profile"/SingletonSocket
rm -rf \
  "$profile/Default/Sessions" \
  "$profile/Default/Session Storage"
rm -f \
  "$profile/Default/Last Tabs" \
  "$profile/Default/Last Session" \
  "$profile/Default/Current Tabs" \
  "$profile/Default/Current Session"

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
