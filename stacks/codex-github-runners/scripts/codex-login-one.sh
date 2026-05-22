#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <container-name>" >&2
  exit 2
fi

container_name="$1"
repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_project="${COMPOSE_PROJECT_NAME:-$(basename "${repo_dir}")}"
image="${CODEX_RUNNER_IMAGE:-codex-github-runner:latest}"
runner_scope="${RUNNER_SCOPE_SLUG:-home-server}"

case "${container_name}" in
  codex-runner-[0-9][0-9]) ;;
  *)
    echo "Unsupported runner container name: ${container_name}" >&2
    echo "Expected a name like codex-runner-01." >&2
    exit 2
    ;;
esac

runner_number="${container_name##*-}"
runner_name="codex-${runner_number}"
holder_id="${runner_scope}-${runner_name}"
home_volume="${compose_project}_codex_runner_${runner_number}_home"
locks_volume="${compose_project}_codex_runner_locks"

tty_flags=()
if [ -t 0 ] && [ -t 1 ]; then
  tty_flags=(-it)
fi

echo "==> Logging Codex into ${container_name} (${home_volume})"
echo "    This creates an independent auth.json for ${container_name}; it does not copy host auth."

docker run --rm "${tty_flags[@]}" \
  --user runner \
  --entrypoint bash \
  -e HOME=/home/runner \
  -e CODEX_HOME=/home/runner/.codex \
  -e FORCE_CODEX_LOGIN="${FORCE_CODEX_LOGIN:-0}" \
  -e AUTH_LOCK_NAME="${holder_id}" \
  -v "${home_volume}:/home/runner/.codex" \
  -v "${locks_volume}:/var/lib/codex-runner/locks" \
  "${image}" \
  -c '
    set -euo pipefail

    mkdir -p "$CODEX_HOME" /var/lib/codex-runner/locks/auth
    chmod 700 "$CODEX_HOME" 2>/dev/null || true

    lock_file="/var/lib/codex-runner/locks/auth/${AUTH_LOCK_NAME}.lock"
    exec 9>"$lock_file"
    if ! flock -n 9; then
      echo "Codex auth lock is busy for ${AUTH_LOCK_NAME}; try again when the runner is idle." >&2
      exit 75
    fi

    verify_auth() {
      python3 - "$CODEX_HOME/auth.json" <<'"'"'PY'"'"'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
tokens = data.get("tokens") or {}
summary = {
    "auth_mode": data.get("auth_mode"),
    "last_refresh": data.get("last_refresh"),
    "has_access_token": bool(tokens.get("access_token")),
    "has_id_token": bool(tokens.get("id_token")),
    "has_refresh_token": bool(tokens.get("refresh_token")),
}
print(summary)
if summary["auth_mode"] != "chatgpt" or not summary["has_refresh_token"]:
    raise SystemExit("auth.json is not valid ChatGPT-managed Codex auth")
PY
    }

    if [ -s "$CODEX_HOME/auth.json" ] && [ "${FORCE_CODEX_LOGIN:-0}" != "1" ]; then
      echo "Existing auth.json found. Set FORCE_CODEX_LOGIN=1 to replace it."
      verify_auth
      exit 0
    fi

    codex -c '"'"'cli_auth_credentials_store="file"'"'"' login --device-auth
    verify_auth
  '
