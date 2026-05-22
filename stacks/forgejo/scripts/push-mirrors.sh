#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd git
require_env GITHUB_OWNER
require_env FORGEJO_SSH_BASE

for repo in "${REPOS[@]}"; do
  ensure_repo "$repo"
  mirror="$ROOT_DIR/mirrors/$repo.git"
  github_url="git@github.com:$GITHUB_OWNER/$repo.git"
  forgejo_url="$(remote_url_for_repo "$repo")"

  echo "== $repo =="
  if [[ ! -d "$mirror" ]]; then
    git clone --mirror "$github_url" "$mirror"
  else
    git -C "$mirror" remote set-url origin "$github_url"
    git -C "$mirror" remote update --prune
  fi

  git -C "$mirror" remote set-url --push origin "$forgejo_url"
  git -C "$mirror" push --mirror
done

