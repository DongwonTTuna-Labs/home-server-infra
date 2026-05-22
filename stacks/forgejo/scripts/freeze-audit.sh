#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd git
require_cmd gh

GITHUB_OWNER="${GITHUB_OWNER:-DongwonTTuna-Labs}"
STAMP="$(timestamp)"
OUT_DIR="$ROOT_DIR/metadata/$STAMP"
mkdir -p "$OUT_DIR"

failed=0

for repo in "${REPOS[@]}"; do
  ensure_repo "$repo"
  path="$(repo_path "$repo")"
  echo "== $repo =="

  branch="$(git -C "$path" branch --show-current)"
  if [[ "$branch" != "main" ]]; then
    echo "ERROR: expected main branch, got $branch"
    failed=1
  fi

  if [[ -n "$(git -C "$path" status --porcelain)" ]]; then
    echo "ERROR: working tree is not clean"
    git -C "$path" status --short
    failed=1
  fi

  git -C "$path" fetch --prune origin
  ahead_behind="$(git -C "$path" rev-list --left-right --count main...origin/main 2>/dev/null || true)"
  if [[ "$ahead_behind" != "0	0" ]]; then
    echo "ERROR: main and origin/main differ: $ahead_behind"
    failed=1
  fi

  gh repo view "$GITHUB_OWNER/$repo" \
    --json name,owner,visibility,isPrivate,defaultBranchRef,isArchived,description,issues,pullRequests,url \
    > "$OUT_DIR/$repo.repo.json"
  gh pr list -R "$GITHUB_OWNER/$repo" --state all --limit 1000 \
    --json number,state,title,headRefName,baseRefName,updatedAt,isDraft,url \
    > "$OUT_DIR/$repo.prs.json"
  gh api "repos/$GITHUB_OWNER/$repo/branches" --paginate > "$OUT_DIR/$repo.branches.json"
  gh api "repos/$GITHUB_OWNER/$repo/tags" --paginate > "$OUT_DIR/$repo.tags.json"

  open_prs="$(gh pr list -R "$GITHUB_OWNER/$repo" --state open --json number --jq 'length')"
  if [[ "$open_prs" != "0" ]]; then
    echo "ERROR: $open_prs open PR(s) remain on GitHub"
    failed=1
  fi
done

echo "metadata: $OUT_DIR"
exit "$failed"

