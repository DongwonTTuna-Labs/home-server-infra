#!/usr/bin/env bash
set -euo pipefail

lock_root="${CODEX_RUNNER_LOCK_DIR:-/var/lib/codex-runner/locks}"
runner_scope="${RUNNER_SCOPE_SLUG:-unknown}"
runner_name="${RUNNER_NAME:-runner}"
holder_id="${runner_scope}-${runner_name}"
state_file="${lock_root}/holders/${holder_id}.state"

if [ ! -f "${state_file}" ]; then
  echo "No Codex runner semaphore state found for ${holder_id}; nothing to release."
  exit 0
fi

holder_pid="$(sed -n 's/^pid=//p' "${state_file}" | head -1 || true)"
slot="$(sed -n 's/^slot=//p' "${state_file}" | head -1 || true)"

if [ -n "${holder_pid}" ] && kill -0 "${holder_pid}" 2>/dev/null; then
  kill "${holder_pid}" 2>/dev/null || true
fi

rm -f "${state_file}"
echo "Released Codex runner semaphore slot ${slot:-unknown} and auth lock for ${holder_id}."
