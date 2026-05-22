#!/usr/bin/env bash
set -euo pipefail

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
compose_project="${COMPOSE_PROJECT_NAME:-$(basename "${repo_dir}")}"
image="${CODEX_RUNNER_IMAGE:-codex-github-runner:latest}"
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

tmp_file="$(mktemp)"
trap 'rm -f "${tmp_file}"' EXIT

printf '%s\n' 'runner|status|auth_mode|last_refresh|access|id|refresh|bytes|sha256'

for container_name in "${containers[@]}"; do
  runner_number="${container_name##*-}"
  runner_name="codex-${runner_number}"
  holder_id="${runner_scope}-${runner_name}"
  home_volume="${compose_project}_codex_runner_${runner_number}_home"
  locks_volume="${compose_project}_codex_runner_locks"

  set +e
  row="$(
    docker run --rm \
      --user runner \
      --entrypoint bash \
      -e HOME=/home/runner \
      -e CODEX_HOME=/home/runner/.codex \
      -e AUTH_LOCK_NAME="${holder_id}" \
      -e RUNNER_CONTAINER_NAME="${container_name}" \
      -v "${home_volume}:/home/runner/.codex:ro" \
      -v "${locks_volume}:/var/lib/codex-runner/locks" \
      "${image}" \
      -c '
        set -euo pipefail
        mkdir -p /var/lib/codex-runner/locks/auth
        lock_file="/var/lib/codex-runner/locks/auth/${AUTH_LOCK_NAME}.lock"
        exec 9>"$lock_file"
        if ! flock -n 9; then
          printf "%s|BUSY|-|-|-|-|-|-|-\n" "$RUNNER_CONTAINER_NAME"
          exit 0
        fi

        python3 - "$RUNNER_CONTAINER_NAME" "$CODEX_HOME/auth.json" <<'"'"'PY'"'"'
import hashlib
import json
import pathlib
import sys

runner = sys.argv[1]
path = pathlib.Path(sys.argv[2])
if not path.is_file():
    print(f"{runner}|MISSING|-|-|-|-|-|-|-")
    raise SystemExit(0)

try:
    raw = path.read_bytes()
    data = json.loads(raw.decode("utf-8"))
except Exception:
    print(f"{runner}|INVALID|-|-|-|-|-|-|-")
    raise SystemExit(0)

tokens = data.get("tokens") or {}
print("|".join([
    runner,
    "OK",
    str(data.get("auth_mode") or "-"),
    str(data.get("last_refresh") or "-"),
    "yes" if tokens.get("access_token") else "no",
    "yes" if tokens.get("id_token") else "no",
    "yes" if tokens.get("refresh_token") else "no",
    str(len(raw)),
    hashlib.sha256(raw).hexdigest(),
]))
PY
      '
  )"
  status=$?
  set -e

  if [ "${status}" -ne 0 ]; then
    row="${container_name}|ERROR|-|-|-|-|-|-|-"
  fi

  printf '%s\n' "${row}"
  printf '%s\n' "${row}" >> "${tmp_file}"
done

duplicate_hashes="$(
  awk -F'|' '$2 == "OK" && $9 != "-" { count[$9]++ } END { for (hash in count) if (count[hash] > 1) print hash ":" count[hash] }' "${tmp_file}"
)"
non_ok_rows="$(
  awk -F'|' '$2 != "OK" { print $0 }' "${tmp_file}"
)"

failed=0

if [ -n "${non_ok_rows}" ]; then
  printf '\nRunner auth entries need attention:\n%s\n' "${non_ok_rows}" >&2
  failed=1
fi

if [ -n "${duplicate_hashes}" ]; then
  printf '\nDuplicate auth.json hashes found:\n%s\n' "${duplicate_hashes}" >&2
  failed=1
fi

exit "${failed}"
