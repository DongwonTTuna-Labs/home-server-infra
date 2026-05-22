#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
host_codex_home="${HOST_CODEX_HOME:-${HOME}/.codex}"
auth_src="${host_codex_home}/auth.json"
compose_project="${COMPOSE_PROJECT_NAME:-$(basename "${repo_dir}")}"

cat >&2 <<'EOF'
WARNING: sync-auth-to-runners.sh copies one host auth.json into runner volumes.
This is unsafe for normal operation because ChatGPT refresh tokens are single-use.
Use scripts/codex-login-one.sh or scripts/codex-login-all.sh to create independent
runner auth instead.
EOF

if [ "${ALLOW_SHARED_AUTH_SYNC:-0}" != "1" ]; then
  echo "Refusing to copy shared auth. Set ALLOW_SHARED_AUTH_SYNC=1 only for emergency reseed/debug." >&2
  exit 2
fi

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

if [ ! -s "${auth_src}" ]; then
  echo "Host Codex auth is missing at ${auth_src}." >&2
  echo "Run scripts/codex-login-all.sh first." >&2
  exit 1
fi

if [ "$#" -gt 0 ]; then
  containers=("$@")
else
  containers=("${default_containers[@]}")
fi

sync_running_container() {
  local container_name="$1"
  local tmp_auth="/home/runner/.codex/.auth-sync-tmp"

  docker cp "${auth_src}" "${container_name}:${tmp_auth}"
  docker exec -u root "${container_name}" sh -lc '
    set -e
    mkdir -p /home/runner/.codex
    install -o runner -g runner -m 0600 /home/runner/.codex/.auth-sync-tmp /home/runner/.codex/auth.json
    rm -f /home/runner/.codex/.auth-sync-tmp
  '
}

sync_home_volume() {
  local container_name="$1"
  local runner_number="${container_name##*-}"
  local volume_name="${compose_project}_codex_runner_${runner_number}_home"

  docker run --rm --user root \
    --entrypoint sh \
    -v "${auth_src}:/tmp/codex-auth.json:ro" \
    -v "${volume_name}:/home/runner/.codex" \
    codex-github-runner:latest \
    -lc '
      set -e
      mkdir -p /home/runner/.codex
      install -o 1001 -g 1001 -m 0600 /tmp/codex-auth.json /home/runner/.codex/auth.json
    '
}

for container_name in "${containers[@]}"; do
  echo "==> Emergency copying shared host Codex auth into ${container_name}"

  if docker ps --format '{{.Names}}' | grep -Fxq "${container_name}"; then
    sync_running_container "${container_name}"
  else
    sync_home_volume "${container_name}"
  fi
done
