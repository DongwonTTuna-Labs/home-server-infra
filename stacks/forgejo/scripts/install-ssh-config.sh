#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_env FORGEJO_SSH_DOMAIN

SSH_DIR="$HOME/.ssh"
CONFIG="$SSH_DIR/config"
IDENTITY_FILE="${FORGEJO_SSH_IDENTITY_FILE:-$HOME/.ssh/id_ed25519_github}"
MARKER_BEGIN="# >>> forgejo cloudflare access >>>"
MARKER_END="# <<< forgejo cloudflare access <<<"

mkdir -p "$SSH_DIR"
chmod 700 "$SSH_DIR"
touch "$CONFIG"
chmod 600 "$CONFIG"

if grep -qF "$MARKER_BEGIN" "$CONFIG"; then
  echo "Forgejo SSH config block already exists in $CONFIG"
  exit 0
fi

cat >> "$CONFIG" <<EOF

$MARKER_BEGIN
Host $FORGEJO_SSH_DOMAIN
  User git
  ProxyCommand cloudflared access ssh --hostname %h
  IdentitiesOnly yes
  IdentityFile $IDENTITY_FILE
$MARKER_END
EOF

echo "updated $CONFIG"

