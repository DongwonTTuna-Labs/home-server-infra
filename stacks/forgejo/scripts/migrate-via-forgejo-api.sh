#!/usr/bin/env bash
set -euo pipefail

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/common.sh"

load_env
require_cmd curl
require_cmd gh
require_env FORGEJO_API_URL
require_env FORGEJO_TOKEN
require_env FORGEJO_OWNER
require_env GITHUB_OWNER

github_token="$(gh auth token)"

for repo in "${REPOS[@]}"; do
  ensure_repo "$repo"
  private="$(gh repo view "$GITHUB_OWNER/$repo" --json isPrivate --jq '.isPrivate')"
  clone_addr="https://github.com/$GITHUB_OWNER/$repo.git"

  echo "== migrating $GITHUB_OWNER/$repo to $FORGEJO_OWNER/$repo =="
  curl --fail-with-body -sS \
    -X POST "$FORGEJO_API_URL/repos/migrate" \
    -H "Authorization: token $FORGEJO_TOKEN" \
    -H "Content-Type: application/json" \
    --data @- <<JSON
{
  "service": "github",
  "clone_addr": "$clone_addr",
  "auth_token": "$github_token",
  "repo_owner": "$FORGEJO_OWNER",
  "repo_name": "$repo",
  "private": $private,
  "mirror": false,
  "issues": true,
  "labels": true,
  "milestones": true,
  "pull_requests": true,
  "releases": true,
  "wiki": true
}
JSON
  echo
done

