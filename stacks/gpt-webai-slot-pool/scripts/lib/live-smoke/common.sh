usage() {
  cat <<'USAGE'
Usage: stacks/gpt-webai-slot-pool/scripts/live-smoke.sh [--case CASE] [--parallel N]

This runner creates real ChatGPT Pro conversations through the Rust lifecycle
and Node Playwright provider.

Cases:
  qa-fast
  qa-full
  sentinel-start-unconfirmed
  live-text
  live-attachment
  live-attachments
  live-artifact-download
  live-artifact-previous
  live-artifact-multiple
  live-resume
  live-parallel-text
  live-parallel-attachment
  live-parallel-attachments
  live-parallel-mixed
  all

Default case: qa-fast
USAGE
}

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

note() {
  printf '== %s ==\n' "$*"
}

shell_quote_command() {
  local arg
  for arg in "$@"; do
    printf '%q ' "$arg"
  done
  printf '\n'
}

write_command_file() {
  local file="$1"
  shift
  shell_quote_command "$@" >"$file"
}

json_get() {
  local file="$1" path="$2"
  python3 - "$file" "$path" <<'PY'
import json, sys
file, path = sys.argv[1:]
with open(file, "r", encoding="utf-8") as handle:
    value = json.load(handle)
for part in path.split("."):
    if not part:
        continue
    if isinstance(value, dict):
        value = value.get(part)
    else:
        value = None
        break
if value is None:
    sys.exit(1)
if isinstance(value, bool):
    print("true" if value else "false")
elif isinstance(value, (dict, list)):
    print(json.dumps(value, sort_keys=True))
else:
    print(value)
PY
}

json_number_ge() {
  local file="$1" path="$2" min="$3" value
  value="$(json_get "$file" "$path" 2>/dev/null || printf '0')"
  [[ "$value" =~ ^[0-9]+$ ]] || value=0
  [[ "$value" -ge "$min" ]]
}

capture_container_status() {
  local out="$1" err="$2"
  set +e
  GPT_WEBAI_SLOT_UID="${GPT_WEBAI_SLOT_UID:-$(id -u)}" \
    GPT_WEBAI_SLOT_GID="${GPT_WEBAI_SLOT_GID:-$(id -g)}" \
    docker compose -f "$repo_root/stacks/gpt-webai-slot-pool/compose.yaml" ps >"$out" 2>"$err"
  printf '%s\n' "$?" >"${out%.out}.rc"
  set -e
}

cleanup_on_exit() {
  local rc="$?"
  trap - EXIT INT TERM
  if [[ "$rc" -ne 0 && "${GPT_WEBAI_SMOKE_EXIT_CLEANUP:-1}" == 1 ]]; then
    local cleanup_dir="$evidence_root/exit-cleanup"
    mkdir -p "$cleanup_dir"
    set +e
    "$lifecycle" cleanup --apply >"$cleanup_dir/cleanup.out" 2>"$cleanup_dir/cleanup.err"
    printf '%s\n' "$?" >"$cleanup_dir/cleanup.rc"
    "$lifecycle" status --json >"$cleanup_dir/status.json" 2>"$cleanup_dir/status.err"
    printf '%s\n' "$?" >"$cleanup_dir/status.rc"
    capture_container_status "$cleanup_dir/containers.out" "$cleanup_dir/containers.err"
    set -e
  fi
  exit "$rc"
}

write_env_file() {
  local file="$1"
  {
    printf 'timestamp=%s\n' "$stamp"
    printf 'repo_root=%s\n' "$repo_root"
    printf 'lifecycle=%s\n' "$lifecycle"
    printf 'state_dir=%s\n' "$state_dir"
    printf 'slot_count=%s\n' "$slot_count"
    printf 'parallel_count=%s\n' "$parallel_count"
    printf 'smoke_poll_timeout_seconds=%s\n' "$smoke_poll_timeout_seconds"
    printf 'cases=%s\n' "${cases[*]}"
    printf 'evidence_root=%s\n' "$evidence_root"
    printf 'expected_send_retry_timeline=1,3,5,10,15\n'
  } >"$file"
}

write_run_metadata() {
  write_command_file "$evidence_root/command.txt" "$0" "${original_args[@]}"
  write_env_file "$evidence_root/env.txt"
  git -C "$repo_root" status --short --branch >"$evidence_root/git-status.txt" 2>"$evidence_root/git-status.err" || true
  "$lifecycle" status --json >"$evidence_root/initial-status.json" 2>"$evidence_root/initial-status.err" || true
  capture_container_status "$evidence_root/containers.initial.out" "$evidence_root/containers.initial.err"
}

assert_clean_status() {
  local file="$1" holders locks
  holders="$(json_get "$file" holders)"
  locks="$(json_get "$file" locks)"
  [[ "$holders" == 0 ]] || fail "expected holders=0 in $file, got $holders"
  [[ "$locks" == 0 ]] || fail "expected locks=0 in $file, got $locks"
}

artifact_dir_for_result() {
  local result="$1" slot run_id
  slot="$(json_get "$result" slotId)"
  run_id="$(json_get "$result" runId)"
  printf '%s/slots/%s/artifacts/%s\n' "$state_dir" "$slot" "$run_id"
}
