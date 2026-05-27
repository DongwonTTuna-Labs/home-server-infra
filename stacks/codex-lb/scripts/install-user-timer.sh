#!/usr/bin/env bash
set -euo pipefail

stack_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
unit_dir="${HOME}/.config/systemd/user"

mkdir -p "${unit_dir}"
systemctl --user stop github-oidc-broker-cleanup.timer 2>/dev/null || true

sed "s#__CODEX_LB_STACK_DIR__#${stack_dir}#g" \
  "${stack_dir}/systemd/github-oidc-broker-cleanup.service" \
  > "${unit_dir}/github-oidc-broker-cleanup.service"
cp "${stack_dir}/systemd/github-oidc-broker-cleanup.timer" \
  "${unit_dir}/github-oidc-broker-cleanup.timer"

systemctl --user daemon-reload
systemctl --user enable --now github-oidc-broker-cleanup.timer
systemctl --user list-timers github-oidc-broker-cleanup.timer
