#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ENV_FILE="$ROOT_DIR/.env"
EXAMPLE_FILE="$ROOT_DIR/.env.example"

if [[ -f "$ENV_FILE" ]]; then
  echo "$ENV_FILE already exists"
  exit 0
fi

password="$(openssl rand -hex 32)"
runner_secret="$(openssl rand -hex 20)"

awk -v password="$password" -v runner_secret="$runner_secret" '
  /^POSTGRES_PASSWORD=/ { print "POSTGRES_PASSWORD=" password; next }
  /^FORGEJO_RUNNER_SECRET=/ { print "FORGEJO_RUNNER_SECRET=" runner_secret; next }
  { print }
' "$EXAMPLE_FILE" > "$ENV_FILE"

chmod 600 "$ENV_FILE"
echo "created $ENV_FILE"
echo "review domains and add FORGEJO_TOKEN before running migration scripts"

