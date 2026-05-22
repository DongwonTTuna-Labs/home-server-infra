#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "${script_dir}/.." && pwd)"
runner_scope="${RUNNER_SCOPE_SLUG:-home-server}"

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

wait_for_docker() {
  local attempt=1
  while [ "${attempt}" -le 30 ]; do
    if docker info >/dev/null 2>&1; then
      return 0
    fi
    sleep 2
    attempt=$((attempt + 1))
  done

  echo "Docker daemon is not ready." >&2
  return 1
}

update_runner() {
  local container_name="$1"
  local runner_number="${container_name##*-}"
  local runner_name="codex-${runner_number}"
  local holder_id="${runner_scope}-${runner_name}"

  echo "==> Updating Codex CLI in ${container_name}"

  if [ "$(docker inspect -f '{{.State.Running}}' "${container_name}" 2>/dev/null || true)" != "true" ]; then
    echo "    skipped: container is not running; entrypoint will update it on next start"
    return 0
  fi

  set +e
  docker exec \
    -e CODEX_CLI_AUTO_UPDATE="${CODEX_CLI_AUTO_UPDATE:-1}" \
    -e CODEX_CLI_VERSION="${CODEX_CLI_VERSION:-latest}" \
    -e AUTH_LOCK_NAME="${holder_id}" \
    -e CODEX_RUNNER_LOCK_DIR="${CODEX_RUNNER_LOCK_DIR:-/var/lib/codex-runner/locks}" \
    "${container_name}" \
    bash -lc '
      set -euo pipefail

      lock_root="${CODEX_RUNNER_LOCK_DIR:-/var/lib/codex-runner/locks}"
      lock_file="${lock_root}/auth/${AUTH_LOCK_NAME}.lock"
      mkdir -p "${lock_root}/auth"

      exec 9>"${lock_file}"
      if ! flock -n 9; then
        echo "SKIP: Codex runner auth lock is busy for ${AUTH_LOCK_NAME}."
        exit 75
      fi

      if [ -x /opt/codex-runner/update-codex-cli.sh ]; then
        /opt/codex-runner/update-codex-cli.sh
      else
        current="$(codex --version 2>/dev/null | awk '"'"'{ print $2 }'"'"' | head -1)"
        latest="$(npm view "@openai/codex@${CODEX_CLI_VERSION:-latest}" version --silent)"
        if [ "${current}" = "${latest}" ]; then
          echo "Codex CLI already current: ${current}"
        else
          echo "Updating Codex CLI ${current:-missing} -> ${latest}"
          npm install -g --no-audit --no-fund --loglevel=error "@openai/codex@${CODEX_CLI_VERSION:-latest}"
          updated="$(codex --version 2>/dev/null | awk '"'"'{ print $2 }'"'"' | head -1)"
          test "${updated}" = "${latest}"
          echo "Codex CLI updated and verified: ${updated}"
        fi
      fi
    '
  local status=$?
  set -e

  case "${status}" in
    0)
      echo "    updated/verified"
      ;;
    75)
      echo "    skipped: runner auth lock is busy"
      ;;
    *)
      echo "    failed" >&2
      return "${status}"
      ;;
  esac
}

wait_for_docker

failed=0
for container_name in "${containers[@]}"; do
  if ! update_runner "${container_name}"; then
    failed=1
  fi
done

exit "${failed}"
