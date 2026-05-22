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

  echo "== $repo =="
  git ls-remote --heads "$forgejo_url" | sort > "$ROOT_DIR/metadata/$repo.forgejo.heads.txt"
  git ls-remote --tags "$forgejo_url" | sort > "$ROOT_DIR/metadata/$repo.forgejo.tags.txt"
  git -C "$path" for-each-ref --format='%(objectname) %(refname)' refs/remotes/origin refs/tags | sort > "$ROOT_DIR/metadata/$repo.local.refs.txt"
  echo "forgejo heads: $(wc -l < "$ROOT_DIR/metadata/$repo.forgejo.heads.txt")"
  echo "forgejo tags:  $(wc -l < "$ROOT_DIR/metadata/$repo.forgejo.tags.txt")"
done

