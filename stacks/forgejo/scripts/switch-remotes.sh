#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd git
require_env FORGEJO_SSH_BASE

for repo in "${REPOS[@]}"; do
  ensure_repo "$repo"
  path="$(repo_path "$repo")"
  forgejo_url="$(remote_url_for_repo "$repo")"
  current_origin="$(git -C "$path" remote get-url origin)"

  echo "== $repo =="
  if git -C "$path" remote get-url github >/dev/null 2>&1; then
    git -C "$path" remote set-url github "$current_origin"
  else
    git -C "$path" remote add github "$current_origin"
  fi
  git -C "$path" remote set-url origin "$forgejo_url"
  git -C "$path" remote -v
done

