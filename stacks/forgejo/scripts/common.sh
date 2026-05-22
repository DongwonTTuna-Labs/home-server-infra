#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROGRAMMING_DIR="/home/dongwonttuna/Documents/Programming"

REPOS=(
  "bioden"
  "polymarket-liquidity-farming-rs"
  "rs-builder-relayer-client"
)

load_env() {
  if [[ -f "$ROOT_DIR/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "$ROOT_DIR/.env"
    set +a
  fi
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

require_env() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "missing required env: $name" >&2
    exit 1
  fi
}

timestamp() {
  date -u +"%Y%m%dT%H%M%SZ"
}

repo_path() {
  printf "%s/%s" "$PROGRAMMING_DIR" "$1"
}

ensure_repo() {
  local repo="$1"
  local path
  path="$(repo_path "$repo")"
  [[ -d "$path/.git" ]] || {
    echo "not a git repository: $path" >&2
    exit 1
  }
}

remote_url_for_repo() {
  local repo="$1"
  require_env FORGEJO_SSH_BASE
  printf "%s/%s.git" "${FORGEJO_SSH_BASE%/}" "$repo"
}

