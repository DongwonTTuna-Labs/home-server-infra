#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stack_root="$(cd -- "$script_dir/.." && pwd -P)"
repo_root="$(cd -- "$stack_root/../.." && pwd -P)"
catalog="$script_dir/qa-live-matrix-cases.r13.tsv"
binary="${GPT_WEBAI_LIFECYCLE_BIN:-$stack_root/target/debug/gpt-webai-lifecycle}"

usage() {
  cat >&2 <<'EOF'
usage:
  qa-live-matrix-r13.sh --iteration N --source-fingerprint SHA
  qa-live-matrix-r13.sh --case Lxx --source-fingerprint SHA
  qa-live-matrix-r13.sh --targeted-case Rxx --repetition N --source-fingerprint SHA
EOF
}

fail() {
  printf 'FAIL qa-live-matrix-r13: %s\n' "$*" >&2
  exit 1
}

usage_error() {
  usage
  exit 2
}

mode=""
iteration=""
case_id=""
targeted_case=""
repetition=""
source_fingerprint=""

set_once() {
  local name="$1" old="$2"
  [[ -z "$old" ]] || usage_error
  printf -v "$name" '%s' "$3"
}

while (( $# > 0 )); do
  case "$1" in
    --iteration)
      (( $# >= 2 )) || usage_error
      set_once iteration "$iteration" "$2"
      shift 2
      ;;
    --case)
      (( $# >= 2 )) || usage_error
      set_once case_id "$case_id" "$2"
      shift 2
      ;;
    --targeted-case)
      (( $# >= 2 )) || usage_error
      set_once targeted_case "$targeted_case" "$2"
      shift 2
      ;;
    --repetition)
      (( $# >= 2 )) || usage_error
      set_once repetition "$repetition" "$2"
      shift 2
      ;;
    --source-fingerprint)
      (( $# >= 2 )) || usage_error
      set_once source_fingerprint "$source_fingerprint" "$2"
      shift 2
      ;;
    *) usage_error ;;
  esac
done

if [[ -n "$iteration" && -z "$case_id" && -z "$targeted_case" && -z "$repetition" ]]; then
  mode="iteration"
  [[ "$iteration" =~ ^[1-9][0-9]*$ ]] || usage_error
elif [[ -n "$case_id" && -z "$iteration" && -z "$targeted_case" && -z "$repetition" ]]; then
  mode="case"
  [[ "$case_id" =~ ^L(0[1-9]|1[0-9]|2[01])$ ]] || usage_error
elif [[ -n "$targeted_case" && -n "$repetition" && -z "$iteration" && -z "$case_id" ]]; then
  mode="targeted"
  [[ "$targeted_case" =~ ^R(0[1-9]|10)$ ]] || usage_error
  [[ "$repetition" =~ ^([1-9]|10)$ ]] || usage_error
else
  usage_error
fi

[[ "$source_fingerprint" =~ ^[0-9a-f]{64}$ ]] || usage_error
[[ -f "$catalog" && ! -L "$catalog" ]] || fail "missing case catalog: $catalog"
[[ -x "$binary" && ! -L "$binary" ]] || fail "lifecycle binary is not executable: $binary"

observed_fingerprint="$(bash "$script_dir/qa-fingerprint-r13.sh" --print)"
[[ "$observed_fingerprint" == "$source_fingerprint" ]] ||
  fail "source fingerprint mismatch expected=$source_fingerprint observed=$observed_fingerprint"
[[ "${GPT_WEBAI_LIVE:-}" == 1 ]] || fail 'GPT_WEBAI_LIVE=1 is required before any live command'

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_base="${GPT_WEBAI_LIVE_EVIDENCE_ROOT:-$repo_root/.omo/evidence/gpt-webai-lifecycle/live-matrix-r13}"
run_tag="$mode"
[[ -n "$iteration" ]] && run_tag+="-$iteration"
[[ -n "$case_id" ]] && run_tag+="-$case_id"
[[ -n "$targeted_case" ]] && run_tag+="-$targeted_case-$repetition"
run_root="$evidence_base/$source_fingerprint/$run_tag-$stamp"
mkdir -p -- "$run_root"
chmod 0700 -- "$run_root"

state_root="${GPT_WEBAI_STATE_ROOT:-${XDG_STATE_HOME:-${HOME}/.local/state}/gpt-webai-lifecycle/r13}"
[[ "$state_root" == /* ]] || fail 'GPT_WEBAI_STATE_ROOT must resolve to an absolute path'

declare -a sessions_to_release=()
declare -a slots_to_release=()
declare -a active_child_pids=()
CASE_ACTIVE=0
FINALIZATION_STARTED=0
FINALIZATION_RC=0
LAST_SESSION=""
LAST_SLOT=""
LAST_RESULT_KIND=""
CASE_STEP=0
CASE_DIR=""
CASE_WORK=""
CASE_ID=""
CASE_COMMAND=""
CASE_ARGV_TEMPLATE=""
CASE_PROMPT_ID=""
CASE_FILES=""
CASE_FAILPOINT=""
CASE_EXPECTED=""
CASE_REPEAT10=""
CASE_LIVE_ONLY=""
REQUEST_ID=""
RUN_ID=""
FENCING_TOKEN=""
UNKNOWN_SESSION_ID=""
PROMPT_PATH=""
FILE_A=""
FILE_B=""
FILE_C=""
declare -a EXPANDED_ARGV=()

lookup_case() {
  local wanted="$1" row count
  row="$(awk -F '\t' -v wanted="$wanted" 'NR > 1 && $1 == wanted { print }' "$catalog")"
  count="$(awk -F '\t' -v wanted="$wanted" 'NR > 1 && $1 == wanted { n++ } END { print n + 0 }' "$catalog")"
  [[ "$count" == 1 ]] || fail "catalog must contain exactly one row for $wanted"
  IFS=$'\t' read -r \
    CASE_ID CASE_COMMAND CASE_ARGV_TEMPLATE CASE_PROMPT_ID CASE_FILES CASE_FAILPOINT \
    CASE_EXPECTED CASE_REPEAT10 CASE_LIVE_ONLY <<<"$row"
}

random_token() {
  od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
}

write_attachment() {
  local target="$1" letter="$2"
  python3 - "$target" "$letter" <<'PY'
import os
import pathlib
import sys

target = pathlib.Path(sys.argv[1])
seed = f"PR72-LIVE-FILE-{sys.argv[2]}\n".encode("ascii")
payload = (seed * ((64 + len(seed) - 1) // len(seed)))[:64]
descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(descriptor, "wb") as stream:
    stream.write(payload)
assert target.stat().st_size == 64
PY
}

prepare_case() {
  local id="$1" ordinal="$2" lower suffix
  lookup_case "$id"
  CASE_DIR="$run_root/$id"
  CASE_WORK="$CASE_DIR/work"
  mkdir -p -- "$CASE_WORK"
  chmod 0700 -- "$CASE_DIR" "$CASE_WORK"
  lower="${id,,}"
  suffix="${source_fingerprint:0:12}-$ordinal"
  REQUEST_ID="pr72-live-$lower-$suffix"
  RUN_ID="pr72-live-$lower-$suffix"
  FENCING_TOKEN="$(random_token)"
  UNKNOWN_SESSION_ID="unknown-$lower-$suffix"
  PROMPT_PATH="$CASE_WORK/prompt.txt"
  FILE_A="$CASE_WORK/file-A.txt"
  FILE_B="$CASE_WORK/file-B.txt"
  FILE_C="$CASE_WORK/file-C.txt"
  if [[ "$CASE_PROMPT_ID" != - ]]; then
    printf 'PR72-LIVE-%s: reply with exactly the single word ACK.' "$id" >"$PROMPT_PATH"
    chmod 0600 -- "$PROMPT_PATH"
  fi
  [[ "$CASE_FILES" != *file-A.txt* ]] || write_attachment "$FILE_A" A
  [[ "$CASE_FILES" != *file-B.txt* ]] || write_attachment "$FILE_B" B
  [[ "$CASE_FILES" != *file-C.txt* ]] || write_attachment "$FILE_C" C
  CASE_STEP=0
  LAST_SESSION=""
  LAST_SLOT=""
  LAST_RESULT_KIND=""
  sessions_to_release=()
  slots_to_release=()
  active_child_pids=()
  CASE_ACTIVE=1
  FINALIZATION_STARTED=0
  FINALIZATION_RC=0
}

expand_template() {
  local token
  local -a raw=()
  read -r -a raw <<<"$CASE_ARGV_TEMPLATE"
  EXPANDED_ARGV=("$CASE_COMMAND")
  for token in "${raw[@]}"; do
    case "$token" in
      '{requestId}') token="$REQUEST_ID" ;;
      '{runId}') token="$RUN_ID" ;;
      '{fencingToken}') token="$FENCING_TOKEN" ;;
      '{promptPath}') token="$PROMPT_PATH" ;;
      '{fileA}') token="$FILE_A" ;;
      '{fileB}') token="$FILE_B" ;;
      '{fileC}') token="$FILE_C" ;;
      '{sessionId}') [[ -n "$LAST_SESSION" ]] || fail "$CASE_ID has no sessionId"; token="$LAST_SESSION" ;;
      '{unknownSessionId}') token="$UNKNOWN_SESSION_ID" ;;
    esac
    EXPANDED_ARGV+=("$token")
  done
}

write_redacted_argv() {
  local target="$1"
  shift
  local redact=0 arg
  {
    printf 'cwd=%q\nargv=' "$stack_root"
    for arg in "$@"; do
      if (( redact )); then
        printf ' %q' '<redacted-fencing-token>'
        redact=0
      else
        printf ' %q' "$arg"
        [[ "$arg" != --fencing-token ]] || redact=1
      fi
    done
    printf '\n'
  } >"$target"
  chmod 0600 -- "$target"
}

validate_envelope_record() {
  local stdout_path="$1" stderr_path="$2" rc_path="$3" expected="$4"
  python3 - "$stdout_path" "$stderr_path" "$rc_path" "$expected" <<'PY'
import json
import pathlib
import sys

stdout_path = pathlib.Path(sys.argv[1])
stderr_path = pathlib.Path(sys.argv[2])
rc_path = pathlib.Path(sys.argv[3])
expected_raw = sys.argv[4]
stdout = stdout_path.read_bytes()
stderr = stderr_path.read_bytes()
rc = int(rc_path.read_text(encoding="ascii"))
try:
    envelope = json.loads(stdout)
except (UnicodeDecodeError, json.JSONDecodeError) as error:
    raise SystemExit(f"stdout is not one JSON envelope: {error}; rc={rc}; stderr={stderr[:400]!r}")
if not isinstance(envelope, dict) or envelope.get("schema") != "gpt-webai.lifecycle.r13.v1":
    raise SystemExit("stdout is not an R13 lifecycle envelope")
expected = expected_raw.split(",")
if envelope.get("resultKind") not in expected:
    raise SystemExit(
        f"resultKind={envelope.get('resultKind')!r} not in expected={expected!r}; rc={rc}"
    )
if stderr:
    raise SystemExit(f"non-usage lifecycle result wrote stderr: {stderr[:400]!r}")
print(envelope.get("sessionId") or "")
print(envelope.get("slotId") or "")
print(envelope["resultKind"])
PY
}

expected_for_command() {
  local command="$1" declared="$2" kind
  local -a kinds=() selected=()
  IFS=',' read -r -a kinds <<<"$declared"
  for kind in "${kinds[@]}"; do
    [[ "$kind" == "$command."* ]] && selected+=("$kind")
  done
  (( ${#selected[@]} > 0 )) || fail "$CASE_ID declares no $command result kind"
  local IFS=,
  printf '%s\n' "${selected[*]}"
}

invoke() {
  local label="$1" expected="$2" failpoint="$3"
  shift 3
  local prefix rc identities
  local -a command=("$binary" "$@")
  CASE_STEP=$((CASE_STEP + 1))
  prefix="$CASE_DIR/$(printf '%03d' "$CASE_STEP")-$label"
  write_redacted_argv "$prefix.argv.txt" "${command[@]}"
  set +e
  if [[ "$failpoint" == - ]]; then
    (cd "$stack_root" && "${command[@]}") >"$prefix.stdout" 2>"$prefix.stderr"
  else
    (cd "$stack_root" && GPT_WEBAI_FAILPOINT="$failpoint" "${command[@]}") \
      >"$prefix.stdout" 2>"$prefix.stderr"
  fi
  rc=$?
  set -e
  printf '%s\n' "$rc" >"$prefix.rc"
  chmod 0600 -- "$prefix.stdout" "$prefix.stderr" "$prefix.rc"
  identities="$(validate_envelope_record "$prefix.stdout" "$prefix.stderr" "$prefix.rc" "$expected")" ||
    fail "$CASE_ID/$label envelope validation failed"
  LAST_SESSION="$(sed -n '1p' <<<"$identities")"
  LAST_SLOT="$(sed -n '2p' <<<"$identities")"
  LAST_RESULT_KIND="$(sed -n '3p' <<<"$identities")"
  [[ -z "$LAST_SESSION" ]] || sessions_to_release+=("$LAST_SESSION")
  [[ -z "$LAST_SLOT" ]] || slots_to_release+=("$LAST_SLOT")
}

invoke_crash() {
  local label="$1" failpoint="$2"
  shift 2
  local prefix rc
  local -a command=("$binary" "$@")
  CASE_STEP=$((CASE_STEP + 1))
  prefix="$CASE_DIR/$(printf '%03d' "$CASE_STEP")-$label"
  write_redacted_argv "$prefix.argv.txt" "${command[@]}"
  set +e
  (cd "$stack_root" && GPT_WEBAI_FAILPOINT="$failpoint" "${command[@]}") \
    >"$prefix.stdout" 2>"$prefix.stderr"
  rc=$?
  set -e
  printf '%s\n' "$rc" >"$prefix.rc"
  chmod 0600 -- "$prefix.stdout" "$prefix.stderr" "$prefix.rc"
  [[ "$rc" == 99 ]] || fail "$CASE_ID/$label failpoint exit=$rc expected=99"
  [[ ! -s "$prefix.stdout" ]] || fail "$CASE_ID/$label failpoint wrote stdout"
  [[ "$(<"$prefix.stderr")" == "failpoint:$failpoint" ]] ||
    fail "$CASE_ID/$label failpoint stderr mismatch"
}

discover_session_for_request() {
  local result
  result="$(python3 - "$state_root" "$REQUEST_ID" <<'PY'
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1]) / "sessions"
request_id = sys.argv[2]
matches = []
if root.is_dir():
    for path in root.glob("*.json"):
        try:
            value = json.loads(path.read_bytes())
        except Exception:
            continue
        if value.get("requestId") == request_id:
            matches.append((value.get("updatedAtMs", 0), value.get("sessionId", ""), value.get("slotId", "")))
if not matches:
    raise SystemExit(1)
_, session_id, slot_id = max(matches)
print(session_id)
print(slot_id)
PY
)" || fail "$CASE_ID could not recover its persisted session"
  LAST_SESSION="$(sed -n '1p' <<<"$result")"
  LAST_SLOT="$(sed -n '2p' <<<"$result")"
  sessions_to_release+=("$LAST_SESSION")
  [[ -z "$LAST_SLOT" ]] || slots_to_release+=("$LAST_SLOT")
}

common_run_argv() {
  local request_id="$1" run_id="$2" token="$3" prompt="$4" expectation="$5"
  EXPANDED_ARGV=(
    run --json --docker-slot-provider --live-send --require-visual-gate
    --request-id "$request_id" --run-id "$run_id" --fencing-token "$token"
    --model pro --effort standard --prompt-file "$prompt"
    --artifact-expectation "$expectation"
  )
}

seed_session() {
  local expectation="${1:-none}"
  [[ -f "$PROMPT_PATH" ]] || fail "$CASE_ID requires a setup prompt"
  common_run_argv "$REQUEST_ID" "$RUN_ID" "$FENCING_TOKEN" "$PROMPT_PATH" "$expectation"
  invoke seed-session 'run.terminal_success,run.terminal_optional_zero' - "${EXPANDED_ARGV[@]}"
  [[ -n "$LAST_SESSION" ]] || fail "$CASE_ID setup run returned no sessionId"
}

resume_current() {
  invoke resume-current 'resume.running,resume.terminal_success,resume.terminal_optional_zero' - \
    resume --json --session "$LAST_SESSION" --fencing-token "$FENCING_TOKEN" --docker-slot-provider
}

corrupt_session_url_for_l16() {
  python3 - "$state_root" "$LAST_SESSION" <<'PY'
import json
import os
import pathlib
import sys

path = pathlib.Path(sys.argv[1]) / "sessions" / f"{sys.argv[2]}.json"
value = json.loads(path.read_bytes())
value["conversationUrl"] = "https://chatgpt.com/"
raw = (json.dumps(value, ensure_ascii=False, separators=(",", ":"), sort_keys=True) + "\n").encode()
tmp = path.with_name(path.name + ".qa-live.tmp")
descriptor = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
with os.fdopen(descriptor, "wb") as stream:
    stream.write(raw)
os.replace(tmp, path)
os.chmod(path, 0o600)
PY
}

wait_for_takeover_window() {
  local seconds="${GPT_WEBAI_LIVE_TAKEOVER_WAIT_SECONDS:-331}"
  [[ "$seconds" =~ ^[1-9][0-9]*$ ]] || fail 'invalid GPT_WEBAI_LIVE_TAKEOVER_WAIT_SECONDS'
  sleep "$seconds"
}

validate_background_record() {
  local prefix="$1" expected="$2" identities
  identities="$(validate_envelope_record "$prefix.stdout" "$prefix.stderr" "$prefix.rc" "$expected")" ||
    fail "$CASE_ID concurrent command failed: $prefix"
  local session slot
  session="$(sed -n '1p' <<<"$identities")"
  slot="$(sed -n '2p' <<<"$identities")"
  [[ -z "$session" ]] || sessions_to_release+=("$session")
  [[ -z "$slot" ]] || slots_to_release+=("$slot")
  LAST_SESSION="$session"
  LAST_SLOT="$slot"
  LAST_RESULT_KIND="$(sed -n '3p' <<<"$identities")"
}

concurrent_runs() {
  local prefix_a="$CASE_DIR/concurrent-a" prefix_b="$CASE_DIR/concurrent-b"
  local prompt_b="$CASE_WORK/prompt-b.txt" request_b="${REQUEST_ID}-b" run_b="${RUN_ID}-b"
  local token_b
  token_b="$(random_token)"
  printf 'PR72-LIVE-%s: reply with exactly the single word ACK.' "$CASE_ID" >"$prompt_b"
  chmod 0600 -- "$prompt_b"
  common_run_argv "${REQUEST_ID}-a" "${RUN_ID}-a" "$FENCING_TOKEN" "$PROMPT_PATH" none
  local -a argv_a=("${EXPANDED_ARGV[@]}")
  common_run_argv "$request_b" "$run_b" "$token_b" "$prompt_b" none
  local -a argv_b=("${EXPANDED_ARGV[@]}")
  write_redacted_argv "$prefix_a.argv.txt" "$binary" "${argv_a[@]}"
  write_redacted_argv "$prefix_b.argv.txt" "$binary" "${argv_b[@]}"
  set +e
  (cd "$stack_root" && "$binary" "${argv_a[@]}") >"$prefix_a.stdout" 2>"$prefix_a.stderr" &
  local pid_a=$!
  active_child_pids+=("$pid_a")
  (cd "$stack_root" && "$binary" "${argv_b[@]}") >"$prefix_b.stdout" 2>"$prefix_b.stderr" &
  local pid_b=$!
  active_child_pids+=("$pid_b")
  wait "$pid_a"; printf '%s\n' "$?" >"$prefix_a.rc"
  wait "$pid_b"; printf '%s\n' "$?" >"$prefix_b.rc"
  active_child_pids=()
  set -e
  chmod 0600 -- "$prefix_a".* "$prefix_b".*
  validate_background_record "$prefix_a" run.terminal_success
  validate_background_record "$prefix_b" run.terminal_success
}

concurrent_unknown_operations() {
  local prefix command
  local -a pids=() prefixes=() expected=()
  for command in show resume download; do
    prefix="$CASE_DIR/unknown-$command"
    prefixes+=("$prefix")
    expected+=("$command.unknown_session")
    local -a argv=("$command" --json --session "$UNKNOWN_SESSION_ID" --fencing-token "$FENCING_TOKEN" --docker-slot-provider)
    [[ "$command" != download ]] || argv+=(--artifact-expectation optional)
    write_redacted_argv "$prefix.argv.txt" "$binary" "${argv[@]}"
    (cd "$stack_root" && "$binary" "${argv[@]}") >"$prefix.stdout" 2>"$prefix.stderr" &
    pids+=("$!")
    active_child_pids+=("$!")
  done
  local index rc
  set +e
  for index in "${!pids[@]}"; do
    wait "${pids[$index]}"; rc=$?
    printf '%s\n' "$rc" >"${prefixes[$index]}.rc"
  done
  active_child_pids=()
  set -e
  for index in "${!prefixes[@]}"; do
    chmod 0600 -- "${prefixes[$index]}".*
    validate_background_record "${prefixes[$index]}" "${expected[$index]}"
  done
}

finalize_case() {
  local cleanup_failed=0 session slot prefix rc
  local release_kinds='release.allocatable,release.cooldown_blocked,release.already_released,release.stop_skipped_owner_alive,release.target_unknown,release.fencing_mismatch,release.takeover_unproven,release.stop_failed,release.cleanup_failed,release.lock_contended'
  declare -A seen_sessions=() seen_slots=()
  set +e
  for session in "${sessions_to_release[@]}"; do
    [[ -n "$session" && -z "${seen_sessions[$session]:-}" ]] || continue
    seen_sessions[$session]=1
    CASE_STEP=$((CASE_STEP + 1))
    prefix="$CASE_DIR/$(printf '%03d' "$CASE_STEP")-final-release-session"
    (cd "$stack_root" && "$binary" release --json --session "$session") \
      >"$prefix.stdout" 2>"$prefix.stderr"
    rc=$?
    printf '%s\n' "$rc" >"$prefix.rc"
    chmod 0600 -- "$prefix".*
    validate_envelope_record \
      "$prefix.stdout" "$prefix.stderr" "$prefix.rc" "$release_kinds" >/dev/null ||
      cleanup_failed=1
    (( rc == 0 )) || cleanup_failed=1
  done
  if (( ${#seen_sessions[@]} == 0 )); then
    for slot in "${slots_to_release[@]}"; do
      [[ -n "$slot" && -z "${seen_slots[$slot]:-}" ]] || continue
      seen_slots[$slot]=1
      CASE_STEP=$((CASE_STEP + 1))
      prefix="$CASE_DIR/$(printf '%03d' "$CASE_STEP")-final-release-slot"
      (cd "$stack_root" && "$binary" release --json --slot "$slot") \
        >"$prefix.stdout" 2>"$prefix.stderr"
      rc=$?
      printf '%s\n' "$rc" >"$prefix.rc"
      chmod 0600 -- "$prefix".*
      validate_envelope_record \
        "$prefix.stdout" "$prefix.stderr" "$prefix.rc" "$release_kinds" >/dev/null ||
        cleanup_failed=1
      (( rc == 0 )) || cleanup_failed=1
    done
  fi
  CASE_STEP=$((CASE_STEP + 1))
  prefix="$CASE_DIR/$(printf '%03d' "$CASE_STEP")-final-status"
  (cd "$stack_root" && "$binary" status --legacy-kv) >"$prefix.stdout" 2>"$prefix.stderr"
  rc=$?
  printf '%s\n' "$rc" >"$prefix.rc"
  chmod 0600 -- "$prefix".*
  (( rc == 0 )) || cleanup_failed=1
  [[ ! -s "$prefix.stderr" ]] || cleanup_failed=1
  grep -qx 'holders=0' "$prefix.stdout" || cleanup_failed=1
  grep -qx 'locks=0' "$prefix.stdout" || cleanup_failed=1
  return "$cleanup_failed"
}

reap_active_children() {
  local pid
  set +e
  for pid in "${active_child_pids[@]}"; do
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid" 2>/dev/null
    fi
    wait "$pid" 2>/dev/null
  done
  active_child_pids=()
}

finalize_active_case() {
  local rc=0
  (( CASE_ACTIVE )) || return 0
  if (( FINALIZATION_STARTED )); then
    return "$FINALIZATION_RC"
  fi
  FINALIZATION_STARTED=1
  set +e
  reap_active_children
  finalize_case
  rc=$?
  FINALIZATION_RC=$rc
  return "$rc"
}

on_exit() {
  local original_rc=$? cleanup_rc=0
  trap - EXIT HUP INT TERM
  set +e
  if (( CASE_ACTIVE )); then
    finalize_active_case
    cleanup_rc=$?
  fi
  if (( original_rc == 0 && cleanup_rc != 0 )); then
    original_rc=$cleanup_rc
  fi
  exit "$original_rc"
}

trap on_exit EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

dispatch_case() {
  case "$CASE_ID" in
    L01)
      expand_template
      invoke preflight "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L02)
      expand_template
      invoke preflight-initial "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      if [[ "$LAST_RESULT_KIND" == preflight.model_correction_required ]]; then
        invoke preflight-corrected preflight.ready - "${EXPANDED_ARGV[@]}"
      fi
      ;;
    L03|L04|L05|L06|L07|L09|L18|R02|R03|R07)
      expand_template
      invoke primary "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L08|R04)
      expand_template
      invoke_crash send-click-crash "$CASE_FAILPOINT" "${EXPANDED_ARGV[@]}"
      discover_session_for_request
      resume_current
      ;;
    L10)
      seed_session none
      expand_template
      invoke show "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L11)
      seed_session none
      expand_template
      invoke resume "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L12|L13|L14)
      seed_session none
      expand_template
      invoke download "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L15)
      expand_template
      invoke unknown-session "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L16)
      seed_session none
      corrupt_session_url_for_l16
      expand_template
      invoke rejected-url "$CASE_EXPECTED" - "${EXPANDED_ARGV[@]}"
      ;;
    L17|R01)
      concurrent_runs
      ;;
    L19)
      common_run_argv "$REQUEST_ID" "$RUN_ID" "$FENCING_TOKEN" "$PROMPT_PATH" none
      invoke_crash terminal-persist-crash "$CASE_FAILPOINT" "${EXPANDED_ARGV[@]}"
      discover_session_for_request
      resume_current
      ;;
    L20|R09)
      seed_session none
      invoke_crash owner-renewal-crash after-session-claim-lease-owner-renewal \
        resume --json --session "$LAST_SESSION" --fencing-token "$FENCING_TOKEN" --docker-slot-provider
      wait_for_takeover_window
      invoke tokenless-release release.allocatable - release --json --session "$LAST_SESSION"
      ;;
    L21)
      local status_expected cleanup_expected status_observed cleanup_observed
      status_expected="$(expected_for_command status "$CASE_EXPECTED")"
      cleanup_expected="$(expected_for_command cleanup "$CASE_EXPECTED")"
      invoke status "$status_expected" - status --json
      status_observed="$LAST_RESULT_KIND"
      invoke cleanup "$cleanup_expected" - cleanup --json --dry-run
      cleanup_observed="$LAST_RESULT_KIND"
      [[ ",$status_expected," == *",$status_observed,"* ]] ||
        fail "$CASE_ID status result is not catalog-bound"
      [[ ",$cleanup_expected," == *",$cleanup_observed,"* ]] ||
        fail "$CASE_ID cleanup result is not catalog-bound"
      ;;
    R05)
      seed_session none
      expand_template
      invoke_crash artifact-click-crash "$CASE_FAILPOINT" "${EXPANDED_ARGV[@]}"
      invoke artifact-recovery download.completed - "${EXPANDED_ARGV[@]}"
      ;;
    R06)
      common_run_argv "$REQUEST_ID" "$RUN_ID" "$FENCING_TOKEN" "$PROMPT_PATH" none
      invoke_crash projection-publish-crash "$CASE_FAILPOINT" "${EXPANDED_ARGV[@]}"
      invoke rebuild "$CASE_EXPECTED" - state-rebuild --json --check-only
      ;;
    R08)
      concurrent_unknown_operations
      ;;
    R10)
      seed_session none
      invoke_crash release-interruption "$CASE_FAILPOINT" release --json --session "$LAST_SESSION"
      invoke release-recovery 'release.allocatable,release.already_released' - \
        release --json --session "$LAST_SESSION"
      ;;
    *) fail "no handler for $CASE_ID" ;;
  esac
}

run_one_case() {
  local id="$1" ordinal="$2" main_rc=0 cleanup_rc=0
  prepare_case "$id" "$ordinal"
  set +e
  dispatch_case
  main_rc=$?
  set -e
  finalize_active_case || cleanup_rc=$?
  set -e
  CASE_ACTIVE=0
  (( main_rc == 0 )) || return "$main_rc"
  (( cleanup_rc == 0 )) || return "$cleanup_rc"
  printf 'PASS qa-live-matrix-r13 case=%s evidence=%s\n' "$id" "$CASE_DIR"
}

case "$mode" in
  iteration)
    ordinal="$iteration"
    while IFS=$'\t' read -r id _; do
      [[ "$id" != caseId && "$id" == L* ]] || continue
      run_one_case "$id" "$ordinal"
    done <"$catalog"
    ;;
  case)
    run_one_case "$case_id" 1
    ;;
  targeted)
    run_one_case "$targeted_case" "$repetition"
    ;;
esac

printf 'PASS qa-live-matrix-r13 mode=%s sourceFingerprint=%s evidence=%s\n' \
  "$mode" "$source_fingerprint" "$run_root"
