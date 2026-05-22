#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd docker

STAMP="$(timestamp)"
OUT_DIR="$ROOT_DIR/backups/$STAMP-forgejo"
mkdir -p "$OUT_DIR"

docker compose --env-file "$ROOT_DIR/.env" -f "$ROOT_DIR/compose.yaml" exec -T db \
  pg_dump -U "${POSTGRES_USER:-forgejo}" "${POSTGRES_DB:-forgejo}" > "$OUT_DIR/postgres.sql"
tar -C "$ROOT_DIR" -czf "$OUT_DIR/forgejo-data.tgz" forgejo

echo "backup written: $OUT_DIR"
echo "restore smoke test is intentionally manual: restore postgres.sql and forgejo-data.tgz into a temporary compose project, then run health checks before trusting backups."

