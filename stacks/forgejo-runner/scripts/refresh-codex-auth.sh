#!/usr/bin/env bash
set -euo pipefail

dind_container="${FORGEJO_RUNNER_DIND_CONTAINER:-forgejo-runner-docker-in-docker-1}"
runner_image="${FORGEJO_RUNNER_JOB_IMAGE:-dongwontuna-labs-runner:latest}"

if ! docker inspect "${dind_container}" >/dev/null 2>&1; then
  echo "DIND container not found: ${dind_container}" >&2
  exit 1
fi

if [ "$(docker inspect -f '{{.State.Running}}' "${dind_container}")" != "true" ]; then
  echo "DIND container is not running: ${dind_container}" >&2
  exit 1
fi

docker exec -i "${dind_container}" sh -s -- "${runner_image}" <<'DIND_SCRIPT'
set -eu

runner_image="$1"
export DOCKER_HOST=tcp://127.0.0.1:2375

attempt=1
while [ "$attempt" -le 30 ]; do
  if docker info >/dev/null 2>&1; then
    break
  fi
  sleep 2
  attempt=$((attempt + 1))
done

if ! docker info >/dev/null 2>&1; then
  echo "DIND Docker daemon is not ready." >&2
  exit 1
fi

docker image inspect "${runner_image}" >/dev/null

docker run --rm -i \
  --entrypoint bash \
  --user root \
  -e HOME=/home/runner \
  -e CODEX_HOME=/home/runner/.codex \
  -v /codex-runner-home:/home/runner/.codex \
  -v /codex-runner-locks:/var/lib/codex-runner/locks \
  "${runner_image}" \
  -s <<'REFRESH_SCRIPT'
set -euo pipefail

auth_file="${CODEX_HOME}/auth.json"
lock_dir="/var/lib/codex-runner/locks/auth"
lock_file="${lock_dir}/forgejo-shared.lock"

mkdir -p "${CODEX_HOME}" "${lock_dir}"
chown 1001:1001 "${CODEX_HOME}" "${lock_dir}" 2>/dev/null || true
chmod 700 "${CODEX_HOME}" 2>/dev/null || true

exec 9>"${lock_file}"
flock 9

if [ -e "${auth_file}" ]; then
  auth_mode="$(stat -c '%a' "${auth_file}" 2>/dev/null || printf '600')"
  if [ "${auth_mode}" = "0" ] || [ "${auth_mode}" = "000" ]; then
    chmod 600 "${auth_file}" 2>/dev/null || true
  fi
fi

if [ ! -s "${auth_file}" ]; then
  echo "Codex auth missing at ${auth_file}." >&2
  exit 1
fi

python3 - "${auth_file}" <<'PY'
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
try:
    os.chown(tmp_path, 1001, 1001)
except PermissionError:
    pass
os.replace(tmp_path, path)

print("|".join([
    "OK",
    data["last_refresh"],
    hashlib.sha256(raw).hexdigest(),
    hashlib.sha256(new_raw).hexdigest(),
]))
PY

chown 1001:1001 "${auth_file}" 2>/dev/null || true
chmod 600 "${auth_file}"

env -u GITHUB_TOKEN -u GIT_AUTH_TOKEN -u GH_TOKEN -u FORGEJO_BOT_TOKEN \
  CODEX_HOME="${CODEX_HOME}" \
  codex -c 'cli_auth_credentials_store="file"' login status

rm -rf -- "${CODEX_HOME}/memories" "${CODEX_HOME}/tmp"
chown -R 1001:1001 "${CODEX_HOME}" 2>/dev/null || true
chmod 700 "${CODEX_HOME}" 2>/dev/null || true
chmod 600 "${auth_file}" 2>/dev/null || true
REFRESH_SCRIPT
DIND_SCRIPT
