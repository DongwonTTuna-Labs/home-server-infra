#!/bin/sh
# Idle-aware updater: only upgrade + restart when opencode has no active session,
# so a running agent task is never interrupted. /session/status == "{}" means idle;
# if busy, re-check every 10 min for up to 2h, else defer to the next timer cycle.
BIN="$HOME/.opencode/bin/opencode"
i=0
while [ "$i" -lt 12 ]; do
  status=$(curl -s -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/session/status 2>/dev/null)
  if [ "$status" = "{}" ]; then
    "$BIN" upgrade || true
    rm -rf "$HOME/.cache/opencode/packages/oh-my-openagent@latest" \
           "$HOME/.config/opencode/node_modules" \
           "$HOME/.config/opencode/bun.lock" 2>/dev/null || true
    systemctl --user restart opencode.service || true
    exit 0
  fi
  i=$((i + 1))
  sleep 600
done
exit 0
