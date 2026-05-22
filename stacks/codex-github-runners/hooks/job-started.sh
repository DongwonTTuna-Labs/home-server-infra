#!/usr/bin/env bash
set -euo pipefail

lock_root="${CODEX_RUNNER_LOCK_DIR:-/var/lib/codex-runner/locks}"
max_parallel="${CODEX_RUNNER_MAX_PARALLEL:-8}"
runner_scope="${RUNNER_SCOPE_SLUG:-unknown}"
runner_name="${RUNNER_NAME:-runner}"
holder_id="${runner_scope}-${runner_name}"
state_dir="${lock_root}/holders"
slot_dir="${lock_root}/slots"
auth_dir="${lock_root}/auth"
state_file="${state_dir}/${holder_id}.state"
auth_lock_file="${auth_dir}/${holder_id}.lock"

mkdir -p "${state_dir}" "${slot_dir}" "${auth_dir}"

release_previous_holder() {
  if [ ! -f "${state_file}" ]; then
    return 0
  fi

  previous_pid="$(sed -n 's/^pid=//p' "${state_file}" | head -1 || true)"
  if [ -n "${previous_pid}" ] && kill -0 "${previous_pid}" 2>/dev/null; then
    kill "${previous_pid}" 2>/dev/null || true
  fi
  rm -f "${state_file}" "${state_file}.tmp"
}

release_previous_holder

if ! [ "${max_parallel}" -ge 1 ] 2>/dev/null; then
  echo "CODEX_RUNNER_MAX_PARALLEL must be a positive integer; got '${max_parallel}'." >&2
  exit 1
fi

echo "Waiting for Codex runner auth lock and semaphore (${max_parallel} host-wide slots)."

while true; do
  (
    exec 8>"${auth_lock_file}"
    if ! flock -n 8; then
      exit 76
    fi

    slot=1
    while [ "${slot}" -le "${max_parallel}" ]; do
      slot_file="${slot_dir}/slot-${slot}.lock"
      exec 9>"${slot_file}"
      if flock -n 9; then
        {
          echo "pid=${BASHPID}"
          echo "slot=${slot}"
          echo "runner=${holder_id}"
          echo "auth_lock=${auth_lock_file}"
          echo "acquired_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        } > "${state_file}.tmp"
        mv "${state_file}.tmp" "${state_file}"

        trap 'rm -f "${state_file}"' EXIT
        while true; do
          sleep 3600
        done
      fi
      slot=$((slot + 1))
    done

    exit 75
  ) &

  holder_pid="$!"
  attempt=1
  while [ "${attempt}" -le 20 ]; do
    if [ -s "${state_file}" ]; then
      slot="$(sed -n 's/^slot=//p' "${state_file}" | head -1 || true)"
      echo "Acquired Codex runner semaphore slot ${slot:-unknown} and auth lock for ${holder_id}."
      if [ "${CODEX_CLI_AUTO_UPDATE:-1}" = "1" ]; then
        if /opt/codex-runner/update-codex-cli.sh; then
          :
        else
          status=$?
          echo "Failed to update Codex CLI before starting the job." >&2
          if kill -0 "${holder_pid}" 2>/dev/null; then
            kill "${holder_pid}" 2>/dev/null || true
            wait "${holder_pid}" 2>/dev/null || true
          fi
          rm -f "${state_file}" "${state_file}.tmp"
          exit "${status}"
        fi
      else
        echo "Codex CLI auto-update disabled."
      fi
      exit 0
    fi

    if ! kill -0 "${holder_pid}" 2>/dev/null; then
      break
    fi

    sleep 0.1
    attempt=$((attempt + 1))
  done

  if kill -0 "${holder_pid}" 2>/dev/null; then
    kill "${holder_pid}" 2>/dev/null || true
    wait "${holder_pid}" 2>/dev/null || true
    echo "Timed out while acquiring Codex runner locks for ${holder_id}; waiting."
  else
    set +e
    wait "${holder_pid}" 2>/dev/null
    status=$?
    set -e
    case "${status}" in
      75)
        echo "All Codex runner semaphore slots are busy; waiting."
        ;;
      76)
        echo "Codex runner auth lock is busy for ${holder_id}; waiting."
        ;;
      *)
        echo "Codex runner lock holder exited unexpectedly with status ${status}; waiting." >&2
        ;;
    esac
  fi

  sleep 5
done
