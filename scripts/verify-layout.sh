#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

required=(
  README.md
  docs/secrets.md
  stacks/codex-lb/README.md
  stacks/codex-lb/compose.yaml
  stacks/codex-lb/seed-oidc-trust.py
  stacks/codex-lb/cloudflared/codex-lb.yml
  stacks/codex-github-runners/compose.yaml
  stacks/codex-github-runners/Dockerfile
  stacks/agent-stack/compose.yml
  stacks/agent-stack/secrets/cloudflared.env.example
  dotfiles/codex/config.toml
  dotfiles/codex/rules/default.rules
)

for path in "${required[@]}"; do
  if [ ! -e "$path" ]; then
    printf 'Missing required path: %s\n' "$path" >&2
    exit 1
  fi
done

scripts/scan-secrets.sh
docker compose -f stacks/codex-lb/compose.yaml config >/dev/null

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
cp -a stacks/codex-github-runners/. "$tmpdir/"
mkdir -p "$tmpdir/state"
printf 'placeholder\n' > "$tmpdir/state/github_pat"
docker compose -f "$tmpdir/compose.yaml" --env-file "$tmpdir/.env.example" config >/dev/null

printf 'Layout verification passed.\n'
