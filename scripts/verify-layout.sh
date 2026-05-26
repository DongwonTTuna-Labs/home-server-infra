#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

required=(
  README.md
  docs/security-model.md
  docs/secrets.md
  docs/restore.md
  docs/migration-status.md
  stacks/codex-lb/compose.yaml
  stacks/codex-lb/cloudflared/codex-lb.yml
  stacks/codex-lb/github-oidc-broker/Dockerfile
  stacks/codex-lb/github-oidc-broker/app/broker.py
  stacks/codex-lb/github-oidc-broker/pyproject.toml
  stacks/codex-lb/github-oidc-broker/tests/test_broker.py
  stacks/forgejo/compose.yaml
  stacks/forgejo/.env.example
  stacks/forgejo-runner/compose.yaml
  stacks/forgejo-runner/config.yaml
  stacks/codex-github-runners/compose.yaml
  stacks/codex-github-runners/Dockerfile
  stacks/agent-stack/compose.yml
  stacks/agent-stack/secrets/cloudflared.env.example
  dotfiles/ssh/config.d/forgejo-cloudflared.conf
  dotfiles/codex/config.toml
  dotfiles/codex/rules/default.rules
  dotfiles/bin/forgejo
)

for path in "${required[@]}"; do
  if [ ! -e "$path" ]; then
    printf 'Missing required path: %s\n' "$path" >&2
    exit 1
  fi
done

if find . -path './.git' -prune -o -path '*/.forgejo/workflows/*' -print | grep -q .; then
  printf 'Application .forgejo workflows must not be mirrored here.\n' >&2
  exit 1
fi

scripts/scan-secrets.sh
docker compose -f stacks/codex-lb/compose.yaml config >/dev/null
docker compose -f stacks/forgejo/compose.yaml --env-file stacks/forgejo/.env.example config >/dev/null

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
cp -a stacks/codex-github-runners/. "$tmpdir/"
mkdir -p "$tmpdir/state"
printf 'placeholder\n' > "$tmpdir/state/github_pat"
docker compose -f "$tmpdir/compose.yaml" --env-file "$tmpdir/.env.example" config >/dev/null

printf 'Layout verification passed.\n'
