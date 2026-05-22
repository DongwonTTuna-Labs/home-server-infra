#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

require_cmd git

STAMP="$(timestamp)"
OUT_DIR="$ROOT_DIR/backups/$STAMP"
mkdir -p "$OUT_DIR"

for repo in "${REPOS[@]}"; do
  ensure_repo "$repo"
  path="$(repo_path "$repo")"
  echo "bundling $repo"
  git -C "$path" fetch --prune --tags origin
  git -C "$path" bundle create "$OUT_DIR/$repo.bundle" --all
  git -C "$path" bundle verify "$OUT_DIR/$repo.bundle"
done

echo "bundles: $OUT_DIR"

