#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

required=(
  README.md
  docs/secrets.md
  .github/actions/setup-codex-relay/action.yml
  stacks/codex-lb/README.md
  stacks/codex-lb/compose.yaml
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

runner_dockerfile=stacks/codex-github-runners/Dockerfile
runner_compose=stacks/codex-github-runners/compose.yaml
runner_readme=stacks/codex-github-runners/README.md

if grep -Eq '(^|[[:space:]])sudo([[:space:]\\]|$)|NOPASSWD|/etc/sudoers' "$runner_dockerfile"; then
  printf 'Runner image must not install sudo or add sudoers rules.\n' >&2
  exit 1
fi

if ! grep -q 'no-new-privileges:true' "$runner_compose"; then
  printf 'Runner compose must keep no-new-privileges:true.\n' >&2
  exit 1
fi

if ! grep -q 'include `sudo`' "$runner_readme" || ! grep -q 'preinstall Codex' "$runner_readme"; then
  printf 'Runner README must document the rootless Codex action contract.\n' >&2
  exit 1
fi

scripts/scan-secrets.sh
docker compose -f stacks/codex-lb/compose.yaml config >/dev/null

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT
cp -a stacks/codex-github-runners/. "$tmpdir/"
mkdir -p "$tmpdir/state"
printf 'placeholder\n' > "$tmpdir/state/github_pat"
docker compose -f "$tmpdir/compose.yaml" --env-file "$tmpdir/.env.example" config >/dev/null

printf 'Layout verification passed.\n'
