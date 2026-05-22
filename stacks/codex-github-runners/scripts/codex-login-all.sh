#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

default_containers=(
  codex-runner-01
  codex-runner-02
  codex-runner-03
  codex-runner-04
  codex-runner-05
  codex-runner-06
  codex-runner-07
  codex-runner-08
)

if [ "$#" -gt 0 ]; then
  containers=("$@")
else
  containers=("${default_containers[@]}")
fi

echo "==> Logging Codex into independent runner auth volumes"
echo "    Each runner needs its own device login. Do not reuse the same auth.json across runners."

for container_name in "${containers[@]}"; do
  "${script_dir}/codex-login-one.sh" "${container_name}"
done

echo "==> Inspecting runner auth uniqueness"
if ! "${script_dir}/inspect-auth-all.sh" "${containers[@]}"; then
  echo "Runner auth inspection failed." >&2
  echo "If duplicate hashes are reported, run FORCE_CODEX_LOGIN=1 $0 to reseed each runner independently." >&2
  exit 1
fi
