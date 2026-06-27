#!/bin/sh
set -eu

bin="$HOME/.opencode/bin/opencode"
i=0

while [ "$i" -lt 12 ]; do
  status=$(curl -fsS -u "opencode:$OPENCODE_SERVER_PASSWORD" http://127.0.0.1:4096/session/status 2>/dev/null || true)
  if [ "$status" = "{}" ]; then
    "$bin" upgrade
    rm -rf "$HOME/.cache/opencode/packages/oh-my-openagent@latest" \
      "$HOME/.config/opencode/node_modules" \
      "$HOME/.config/opencode/bun.lock"
    systemctl --user restart opencode.service
    exit 0
  fi
  i=$((i + 1))
  sleep 600
done

printf 'OpenCode is still busy; deferring update to the next timer cycle.\n' >&2
