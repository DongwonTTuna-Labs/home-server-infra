#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env

uid="${FORGEJO_UID:-1000}"
gid="${FORGEJO_GID:-1000}"

mkdir -p "$ROOT_DIR/forgejo" "$ROOT_DIR/postgres" "$ROOT_DIR/runner"
if [[ ! -d "$ROOT_DIR/runner/data" ]]; then
  mkdir -p "$ROOT_DIR/runner/data"
fi

if [[ "$(stat -c '%u:%g' "$ROOT_DIR/forgejo")" != "$uid:$gid" ]]; then
  docker run --rm -v "$ROOT_DIR/forgejo:/target" alpine:3.22 sh -c "chown -R $uid:$gid /target && chmod 700 /target"
else
  chmod 700 "$ROOT_DIR/forgejo"
fi

docker run --rm -v "$ROOT_DIR/runner/data:/target" alpine:3.22 sh -c "mkdir -p /target/.cache && chown -R 1001:1001 /target && chmod -R ug+rwX /target"

echo "prepared Forgejo directories"
