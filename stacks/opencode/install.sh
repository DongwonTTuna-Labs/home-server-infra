#!/usr/bin/env bash
# Deploy the native opencode stack as user systemd services (no sudo; relies on
# `loginctl enable-linger` being on for this user). Run as the home-server user.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
CFG="$HOME/.config/opencode"
UNITS="$HOME/.config/systemd/user"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"

# 1. opencode binary (native, self-updatable via `opencode upgrade`)
[ -x "$HOME/.opencode/bin/opencode" ] || curl -fsSL https://opencode.ai/install | bash

# 2. config mirror + tunnel config + updater script
mkdir -p "$CFG"
cp -f "$HERE/opencode/opencode.json"            "$CFG/opencode.json"
cp -f "$HERE/opencode/oh-my-openagent.jsonc"    "$CFG/oh-my-openagent.jsonc"
cp -f "$HERE/opencode/AGENTS.md"                "$CFG/AGENTS.md"
cp -f "$HERE/systemd/cloudflared-opencode.yml"  "$CFG/cloudflared-opencode.yml"
install -m 755 "$HERE/systemd/opencode-update.sh" "$CFG/opencode-update.sh"

# 3. secrets must already exist (host-only, not committed)
if [ ! -f "$CFG/opencode.env" ]; then
  echo "ERROR: create $CFG/opencode.env from $HERE/.env.example first" >&2
  exit 1
fi
chmod 600 "$CFG/opencode.env"

# 4. user systemd units
mkdir -p "$UNITS"
cp -f "$HERE/systemd/opencode.service"             "$UNITS/opencode.service"
cp -f "$HERE/systemd/cloudflared-opencode.service" "$UNITS/cloudflared-opencode.service"
cp -f "$HERE/systemd/opencode-update.service"      "$UNITS/opencode-update.service"
cp -f "$HERE/systemd/opencode-update.timer"        "$UNITS/opencode-update.timer"

systemctl --user daemon-reload
systemctl --user enable --now opencode.service
systemctl --user enable --now cloudflared-opencode.service
systemctl --user enable --now opencode-update.timer
echo "opencode native stack installed and started."
