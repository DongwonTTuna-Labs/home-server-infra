#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd gh
require_env GITHUB_OWNER
require_env FORGEJO_ROOT_URL

dry_run=false
if [[ "${1:-}" == "--dry-run" ]]; then
  dry_run=true
fi

for repo in "${REPOS[@]}"; do
  new_description="${GITHUB_READONLY_DESCRIPTION_PREFIX:-Migrated to Forgejo:} ${FORGEJO_ROOT_URL%/}/${FORGEJO_OWNER:-DongwonTTuna-Labs}/$repo"
  echo "== $GITHUB_OWNER/$repo =="
  echo "description: $new_description"
  echo "archive: true"
  if [[ "$dry_run" == "false" ]]; then
    gh repo edit "$GITHUB_OWNER/$repo" --description "$new_description"
    gh repo archive "$GITHUB_OWNER/$repo" --yes
  fi
done

