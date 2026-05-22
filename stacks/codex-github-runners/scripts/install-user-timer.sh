#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
unit_dir="${HOME}/.config/systemd/user"

mkdir -p "${unit_dir}"
sed "s#__CODEX_RUNNER_DIR__#${repo_dir}#g" \
  "${repo_dir}/systemd/codex-runner-auth-refresh.service" \
  > "${unit_dir}/codex-runner-auth-refresh.service"
cp "${repo_dir}/systemd/codex-runner-auth-refresh.timer" \
  "${unit_dir}/codex-runner-auth-refresh.timer"

systemctl --user daemon-reload
systemctl --user enable --now codex-runner-auth-refresh.timer
systemctl --user list-timers codex-runner-auth-refresh.timer
