#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_dir="$(cd "${script_dir}/.." && pwd)"
compose_project="${COMPOSE_PROJECT_NAME:-$(basename "${repo_dir}")}"
image="${CODEX_RUNNER_IMAGE:-codex-github-runner:latest}"
runner_scope="${RUNNER_SCOPE_SLUG:-home-server}"
maintenance_dir="${repo_dir}/state/codex-auth-refresh"

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

mkdir -p "${maintenance_dir}"

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

refresh_runner() {
  local container_name="$1"
  local runner_number="${container_name##*-}"
  local runner_name="codex-${runner_number}"
  local holder_id="${runner_scope}-${runner_name}"
  local home_volume="${compose_project}_codex_runner_${runner_number}_home"
  local locks_volume="${compose_project}_codex_runner_locks"
  local log_file="${maintenance_dir}/refresh-${container_name}.log"

  echo "==> Refreshing ${container_name} (${home_volume})"

  set +e
  docker run --rm \
    --user runner \
    --entrypoint bash \
    -e HOME=/home/runner \
    -e CODEX_HOME=/home/runner/.codex \
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
        echo "SKIP: Codex auth lock is busy for ${AUTH_LOCK_NAME}."
        exit 75
      fi

      if [ ! -s "$CODEX_HOME/auth.json" ]; then
        echo "ERROR: missing $CODEX_HOME/auth.json. Run scripts/codex-login-one.sh first." >&2
        exit 2
      fi

      python3 - "$CODEX_HOME/auth.json" <<'"'"'PY'"'"'
import datetime
import hashlib
import json
import os
import pathlib
import sys
import urllib.error
import urllib.parse
import urllib.request

path = pathlib.Path(sys.argv[1])
raw = path.read_bytes()
data = json.loads(raw.decode("utf-8"))
tokens = data.get("tokens") or {}
old_refresh_token = tokens.get("refresh_token")
if data.get("auth_mode") != "chatgpt" or not old_refresh_token:
    raise SystemExit("auth.json is not valid ChatGPT-managed Codex auth")

request_body = urllib.parse.urlencode({
    "grant_type": "refresh_token",
    "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
    "refresh_token": old_refresh_token,
}).encode("utf-8")

request = urllib.request.Request(
    "https://auth.openai.com/oauth/token",
    data=request_body,
    headers={
        "Accept": "application/json",
        "Content-Type": "application/x-www-form-urlencoded",
    },
    method="POST",
)

try:
    with urllib.request.urlopen(request, timeout=60) as response:
        refreshed = json.loads(response.read().decode("utf-8"))
except urllib.error.HTTPError as exc:
    body = exc.read().decode("utf-8", "replace")[:1000]
    raise SystemExit(f"OAuth token refresh failed with HTTP {exc.code}: {body}")
except urllib.error.URLError as exc:
    raise SystemExit(f"OAuth token refresh failed: {exc}")

missing = [
    key
    for key in ("access_token", "id_token", "refresh_token")
    if not refreshed.get(key)
]
if missing:
    raise SystemExit(
        f"OAuth token refresh response missed required fields: {missing}; "
        f"response keys={sorted(refreshed)}"
    )

if refreshed["refresh_token"] == old_refresh_token:
    raise SystemExit("OAuth token refresh returned the same refresh_token")

tokens["access_token"] = refreshed["access_token"]
tokens["id_token"] = refreshed["id_token"]
tokens["refresh_token"] = refreshed["refresh_token"]
if refreshed.get("account_id"):
    tokens["account_id"] = refreshed["account_id"]

data["auth_mode"] = "chatgpt"
data["tokens"] = tokens
data["last_refresh"] = (
    datetime.datetime.now(datetime.timezone.utc)
    .isoformat()
    .replace("+00:00", "Z")
)

new_raw = (json.dumps(data, indent=2, sort_keys=True) + "\n").encode("utf-8")
tmp_path = path.with_name(".auth.json.tmp")
tmp_path.write_bytes(new_raw)
os.chmod(tmp_path, 0o600)
os.replace(tmp_path, path)

print("|".join([
    "OK",
    data["last_refresh"],
    hashlib.sha256(raw).hexdigest(),
    hashlib.sha256(new_raw).hexdigest(),
]))
PY

      if [ "${CODEX_REFRESH_SMOKE_EXEC:-0}" = "1" ]; then
        codex -c '"'"'cli_auth_credentials_store="file"'"'"' exec \
          --skip-git-repo-check \
          --sandbox read-only \
          --output-last-message "$CODEX_HOME/.auth-refresh-last-message" \
          "Reply with exactly: OK."

        grep -q "OK" "$CODEX_HOME/.auth-refresh-last-message"
      fi
    ' >"${log_file}" 2>&1
  local status=$?
  set -e

  case "${status}" in
    0)
      echo "    refreshed"
      ;;
    75)
      echo "    skipped: runner auth lock is busy"
      ;;
    *)
      echo "    failed: see ${log_file}" >&2
      return "${status}"
      ;;
  esac
}

wait_for_docker

preflight_log="${maintenance_dir}/inspect-before-refresh.log"
set +e
"${script_dir}/inspect-auth-all.sh" "${containers[@]}" >"${preflight_log}" 2>&1
preflight_status=$?
set -e

if [ "${preflight_status}" -ne 0 ]; then
  if grep -q "Duplicate auth.json hashes found" "${preflight_log}"; then
    echo "Refusing to refresh because runner auth files are duplicated." >&2
    echo "Run FORCE_CODEX_LOGIN=1 scripts/codex-login-all.sh to reseed each runner independently." >&2
    echo "See ${preflight_log} for details." >&2
    exit 1
  fi
  if grep -Eq '\|(MISSING|INVALID|ERROR)\|' "${preflight_log}"; then
    echo "Refusing to refresh because at least one runner auth entry is missing or invalid." >&2
    echo "See ${preflight_log} for details." >&2
    exit 1
  fi
fi

failed=0
for container_name in "${containers[@]}"; do
  if ! refresh_runner "${container_name}"; then
    failed=1
  fi
done

exit "${failed}"
