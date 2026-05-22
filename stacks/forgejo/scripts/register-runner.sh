#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd docker
require_env FORGEJO_RUNNER_SECRET
require_env FORGEJO_RUNNER_INSTANCE_URL
require_env FORGEJO_RUNNER_NAME
require_env FORGEJO_RUNNER_LABELS
require_env FORGEJO_OWNER

if [[ ! "$FORGEJO_RUNNER_SECRET" =~ ^[0-9a-fA-F]{40}$ ]]; then
  echo "FORGEJO_RUNNER_SECRET must be exactly 40 hex characters" >&2
  exit 1
fi

if [[ "$FORGEJO_RUNNER_INSTANCE_URL" =~ ^https?://forgejo(:|/|$) ]]; then
  echo "FORGEJO_RUNNER_INSTANCE_URL must be a URL reachable from job containers, such as https://git.dongwontuna.net" >&2
  exit 1
fi

"$ROOT_DIR/scripts/prepare-dirs.sh"

docker compose --env-file "$ROOT_DIR/.env" -f "$ROOT_DIR/compose.yaml" exec forgejo \
  forgejo forgejo-cli actions register \
  --name "$FORGEJO_RUNNER_NAME" \
  --scope "$FORGEJO_OWNER" \
  --secret "$FORGEJO_RUNNER_SECRET" \
  --labels "$FORGEJO_RUNNER_LABELS"

docker run --rm \
  -u 1001:1001 \
  --network forgejo_forgejo \
  -v "$ROOT_DIR/runner/data:/data" \
  data.forgejo.org/forgejo/runner:4.0.0 \
  forgejo-runner create-runner-file \
  --instance "$FORGEJO_RUNNER_INSTANCE_URL" \
  --name "$FORGEJO_RUNNER_NAME" \
  --secret "$FORGEJO_RUNNER_SECRET" \
  --connect

echo "runner registered; start it with:"
echo "docker compose --env-file $ROOT_DIR/.env -f $ROOT_DIR/runner.compose.yaml up -d"
