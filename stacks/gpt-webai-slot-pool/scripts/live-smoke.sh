#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
lifecycle="${GPT_WEBAI_LIFECYCLE:-$repo_root/stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle}"
gptpro="${GPT_WEBAI_GPTPRO:-/home/dongwonttuna/.local/bin/gptpro}"
state_dir="${GPT_WEBAI_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-lifecycle}"
slot_count="${GPT_WEBAI_SLOT_COUNT:-10}"
parallel_count=5
parallel_explicit=0
cases=()

export GPT_WEBAI_STATE_DIR="$state_dir"
export PATH="/home/dongwonttuna/.local/bin:$PATH"

usage() {
  cat <<'EOF'
Usage: stacks/gpt-webai-slot-pool/scripts/live-smoke.sh [--case CASE] [--parallel N]

This runner always creates real ChatGPT Pro conversations.

Cases:
  qa-fast
  qa-full
  live-text
  live-attachment
  live-attachments
  live-resume
  live-parallel-text
  live-parallel-attachment
  live-parallel-attachments
  live-parallel-mixed
  all

Default cases:
  qa-fast
EOF
}

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

note() {
  printf '== %s ==\n' "$*"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --case)
      [[ $# -ge 2 ]] || fail '--case requires a value'
      cases+=("$2")
      shift 2
      ;;
    --parallel)
      [[ $# -ge 2 ]] || fail '--parallel requires a value'
      parallel_count="$2"
      parallel_explicit=1
      [[ "$parallel_count" =~ ^[0-9]+$ && "$parallel_count" -ge 1 && "$parallel_count" -le "$slot_count" ]] || fail "--parallel must be 1..$slot_count"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

if [[ "${#cases[@]}" -eq 0 ]]; then
  cases=(qa-fast)
fi

expanded_cases=()
for case_name in "${cases[@]}"; do
  if [[ "$case_name" == all ]]; then
    expanded_cases+=(qa-full)
  else
    expanded_cases+=("$case_name")
  fi
done
cases=("${expanded_cases[@]}")

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_root="${GPT_WEBAI_SMOKE_EVIDENCE:-$repo_root/.omo/evidence/gpt-webai-slot-pool-live-smoke/$stamp}"
mkdir -p "$evidence_root"
smoke_bin="$evidence_root/bin"
mkdir -p "$smoke_bin"
ln -sf "$lifecycle" "$smoke_bin/gpt-webai-lifecycle"
export PATH="$smoke_bin:/home/dongwonttuna/.local/bin:$PATH"

json_get() {
  local file="$1" path="$2"
  python3 - "$file" "$path" <<'PY'
import json
import sys

file, path = sys.argv[1:]
with open(file, "r", encoding="utf-8") as handle:
    data = json.load(handle)

value = data
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
if isinstance(value, (dict, list)):
    print(json.dumps(value, sort_keys=True))
else:
    print(value)
PY
}

record_for_session() {
  local session_id="$1"
  grep -R -l "sessionId=$session_id" "$state_dir/sessions" 2>/dev/null | head -n 1
}

record_value() {
  local file="$1" key="$2"
  awk -F= -v key="$key" '$1 == key {print substr($0, length(key) + 2); exit}' "$file"
}

assert_final_ready() {
  local status_file="$1" n i key
  "$lifecycle" status >"$status_file"
  grep -F 'holders=0' "$status_file" >/dev/null || fail 'holders were not released'
  grep -F 'locks=0' "$status_file" >/dev/null || fail 'locks were not released'
  for n in $(seq 1 "$slot_count"); do
    i="$(printf '%02d' "$n")"
    key="slot_${i}_status=ready"
    grep -F "$key" "$status_file" >/dev/null || fail "missing final ready status: $key"
  done
}

assert_live_result() {
  local out="$1" token="$2" session_id record
  python3 -m json.tool "$out" >/dev/null
  [[ "$(json_get "$out" status)" == complete ]] || fail "not complete: $out"
  json_get "$out" answerText | grep -F "$token" >/dev/null || fail "answer missing token: $token"
  json_get "$out" url | grep -E '^https://chatgpt\.com/c/' >/dev/null || fail "missing conversation URL"
  session_id="$(json_get "$out" sessionId)"
  record="$(record_for_session "$session_id")"
  [[ -n "$record" && -f "$record" ]] || fail "missing session record for $session_id"
  grep -F 'kind=pro' "$record" >/dev/null || fail "record is not pro: $record"
  grep -F 'model=pro' "$record" >/dev/null || fail "record model is not pro: $record"
  grep -F 'effort=extended' "$record" >/dev/null || fail "record effort is not extended: $record"
  grep -F 'status=done' "$record" >/dev/null || fail "record not done: $record"
  grep -E '^slotId=slot-[0-9][0-9]$' "$record" >/dev/null || fail "record missing slotId: $record"
  grep -E '^conversationUrl=https://chatgpt\.com/c/' "$record" >/dev/null || fail "record missing conversationUrl: $record"
}

run_gptpro_text() {
  local case_dir="$1" token="$2" prompt out err rc
  prompt="Reply with exactly: $token"
  out="$case_dir/gptpro.out"
  err="$case_dir/gptpro.err"
  set +e
  "$gptpro" "$prompt" >"$out" 2>"$err"
  rc=$?
  set -e
  printf '%s\n' "$rc" >"$case_dir/gptpro.rc"
  [[ "$rc" -eq 0 ]] || fail "gptpro text failed rc=$rc; see $err"
  assert_live_result "$out" "$token"
}

run_gptpro_attachments() {
  local case_dir="$1" token="$2" file_count="$3" prompt out err rc session_id record index canary
  local -a args=() expected_lines=()
  [[ "$file_count" =~ ^[0-9]+$ && "$file_count" -ge 1 ]] || fail 'file_count must be positive'

  : >"$case_dir/sha256.txt"
  for index in $(seq 1 "$file_count"); do
    canary="$case_dir/ATTACHMENT_CANARY_${index}.md"
    cat >"$canary" <<EOF
# Live Attachment Canary $index

CANARY_TOKEN_$index: ${token}_F${index}

If you can read this attached file, include this exact line in your answer:
CANARY_OK_$index: ${token}_F${index}

If you cannot read the attached file, reply exactly:
ATTACHMENT_MISSING
EOF
    sha256sum "$canary" >>"$case_dir/sha256.txt"
    args+=(--file "$canary")
    expected_lines+=("CANARY_OK_$index: ${token}_F${index}")
  done

  prompt="Live attachment smoke test. Read every attached ATTACHMENT_CANARY file. Reply with exactly the CANARY_OK lines from all attached files, one per line, and nothing else. If any attached file is unavailable, reply exactly ATTACHMENT_MISSING."
  out="$case_dir/gptpro.out"
  err="$case_dir/gptpro.err"
  set +e
  "$gptpro" "${args[@]}" "$prompt" >"$out" 2>"$err"
  rc=$?
  set -e
  printf '%s\n' "$rc" >"$case_dir/gptpro.rc"
  [[ "$rc" -eq 0 ]] || fail "gptpro attachment failed rc=$rc; see $err"
  for expected in "${expected_lines[@]}"; do
    assert_live_result "$out" "$expected"
  done
  if grep -F 'ATTACHMENT_MISSING' "$out" >/dev/null; then
    fail "provider reported ATTACHMENT_MISSING"
  fi
  session_id="$(json_get "$out" sessionId)"
  record="$(record_for_session "$session_id")"
  grep -F "attachmentCount=$file_count" "$record" >/dev/null || fail "record missing attachmentCount=$file_count: $record"
}

run_gptpro_attachment() {
  run_gptpro_attachments "$1" "$2" 1
}

case_live_text() {
  note live-text
  local case_dir token
  case_dir="$evidence_root/live-text"
  mkdir -p "$case_dir"
  token="LIVE_TEXT_SMOKE_${stamp}_T1"
  printf '%s\n' "$token" >"$case_dir/token.txt"
  run_gptpro_text "$case_dir" "$token"
  assert_final_ready "$case_dir/final-status.out"
}

case_live_attachment() {
  note live-attachment
  local case_dir token
  case_dir="$evidence_root/live-attachment"
  mkdir -p "$case_dir"
  token="LIVE_ATTACHMENT_SMOKE_${stamp}_A1"
  printf '%s\n' "$token" >"$case_dir/token.txt"
  run_gptpro_attachment "$case_dir" "$token"
  assert_final_ready "$case_dir/final-status.out"
}

case_live_attachments() {
  note live-attachments
  local case_dir token
  case_dir="$evidence_root/live-attachments"
  mkdir -p "$case_dir"
  token="LIVE_ATTACHMENTS_SMOKE_${stamp}_M3"
  printf '%s\n' "$token" >"$case_dir/token.txt"
  run_gptpro_attachments "$case_dir" "$token" 3
  assert_final_ready "$case_dir/final-status.out"
}

case_live_resume() {
  note live-resume
  local case_dir token sid out err rc
  case_dir="$evidence_root/live-resume"
  mkdir -p "$case_dir"
  token="LIVE_RESUME_SMOKE_${stamp}_R1"
  printf '%s\n' "$token" >"$case_dir/token.txt"
  run_gptpro_text "$case_dir" "$token"
  sid="$(json_get "$case_dir/gptpro.out" sessionId)"
  out="$case_dir/resume.out"
  err="$case_dir/resume.err"
  set +e
  "$lifecycle" resume --kind pro --session "$sid" >"$out" 2>"$err"
  rc=$?
  set -e
  printf '%s\n' "$rc" >"$case_dir/resume.rc"
  [[ "$rc" -eq 0 ]] || fail "resume failed rc=$rc; see $err"
  python3 -m json.tool "$out" >/dev/null
  json_get "$out" answerText | grep -F "$token" >/dev/null || fail 'resume answer did not contain original token'
  assert_final_ready "$case_dir/final-status.out"
}

parallel_worker_text() {
  local index="$1" case_dir="$2" token
  mkdir -p "$case_dir/worker-$index"
  token="LIVE_PARALLEL_TEXT_${stamp}_${index}"
  printf '%s\n' "$token" >"$case_dir/worker-$index/token.txt"
  run_gptpro_text "$case_dir/worker-$index" "$token"
}

parallel_worker_attachments() {
  local index="$1" case_dir="$2" file_count="$3" token
  mkdir -p "$case_dir/worker-$index"
  token="LIVE_PARALLEL_ATTACHMENT_${stamp}_${index}_N${file_count}"
  printf '%s\n' "$token" >"$case_dir/worker-$index/token.txt"
  run_gptpro_attachments "$case_dir/worker-$index" "$token" "$file_count"
}

case_live_parallel() {
  local mode="$1" width="$2" file_count="${3:-1}"
  local case_dir pids=() i pid failure=0 session_file slot_file session_id record slot_id
  note "live-parallel-$mode width=$width files=$file_count"
  case_dir="$evidence_root/live-parallel-$mode-w$width"
  mkdir -p "$case_dir"
  for i in $(seq 1 "$width"); do
    mkdir -p "$case_dir/worker-$i"
    case "$mode" in
      text)
        (parallel_worker_text "$i" "$case_dir") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
        ;;
      attachment|attachments)
        (parallel_worker_attachments "$i" "$case_dir" "$file_count") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
        ;;
      mixed)
        if (( i % 2 == 1 )); then
          (parallel_worker_text "$i" "$case_dir") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
        else
          (parallel_worker_attachments "$i" "$case_dir" "$file_count") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
        fi
        ;;
      *) fail "unknown parallel mode: $mode" ;;
    esac
    pids+=("$!")
  done
  for pid in "${pids[@]}"; do
    if ! wait "$pid"; then
      failure=1
    fi
  done
  "$lifecycle" status >"$case_dir/final-status.out" || true
  [[ "$failure" -eq 0 ]] || fail "one or more parallel $mode workers failed; final status captured at $case_dir/final-status.out"

  session_file="$case_dir/session-ids.txt"
  slot_file="$case_dir/slot-ids.txt"
  : >"$session_file"
  : >"$slot_file"
  for i in $(seq 1 "$width"); do
    session_id="$(json_get "$case_dir/worker-$i/gptpro.out" sessionId)"
    printf '%s\n' "$session_id" >>"$session_file"
    record="$(record_for_session "$session_id")"
    [[ -n "$record" && -f "$record" ]] || fail "missing session record for $session_id"
    slot_id="$(record_value "$record" slotId)"
    [[ "$slot_id" =~ ^slot-[0-9][0-9]$ ]] || fail "record missing slotId for $session_id"
    printf '%s\n' "$slot_id" >>"$slot_file"
  done
  python3 - "$session_file" "$slot_file" "$width" <<'PY'
import sys

session_path, slot_path, expected_count = sys.argv[1], sys.argv[2], int(sys.argv[3])
with open(session_path, "r", encoding="utf-8") as handle:
    sessions = [line.strip() for line in handle if line.strip()]
with open(slot_path, "r", encoding="utf-8") as handle:
    slots = [line.strip() for line in handle if line.strip()]
if len(sessions) != expected_count:
    raise SystemExit(f"expected {expected_count} sessions, got {len(sessions)}")
if len(set(sessions)) != len(sessions):
    raise SystemExit(f"duplicate session ids: {sessions!r}")
if len(slots) != expected_count:
    raise SystemExit(f"expected {expected_count} slots, got {len(slots)}")
if len(set(slots)) != len(slots):
    raise SystemExit(f"parallel workers reused slot ids: {slots!r}")
PY
  assert_final_ready "$case_dir/final-status.out"
}

case_qa_fast() {
  case_live_text
  case_live_attachment
  case_live_attachments
  case_live_resume
  case_live_parallel text 5
  case_live_parallel attachment 5 1
  case_live_parallel attachments 5 3
  case_live_parallel mixed 5 3
}

case_qa_full() {
  case_live_text
  case_live_attachment
  case_live_attachments
  case_live_resume
  for width in 1 5 10; do
    case_live_parallel text "$width"
    case_live_parallel attachment "$width" 1
    case_live_parallel attachments "$width" 3
  done
  case_live_parallel mixed 10 3
}

for case_name in "${cases[@]}"; do
  case "$case_name" in
    qa-fast) case_qa_fast ;;
    qa-full) case_qa_full ;;
    live-text) case_live_text ;;
    live-attachment) case_live_attachment ;;
    live-attachments) case_live_attachments ;;
    live-resume) case_live_resume ;;
    live-parallel-text) case_live_parallel text "$parallel_count" ;;
    live-parallel-attachment) case_live_parallel attachment "$parallel_count" 1 ;;
    live-parallel-attachments) case_live_parallel attachments "$parallel_count" 3 ;;
    live-parallel-mixed) case_live_parallel mixed "$parallel_count" 3 ;;
    *) fail "unknown case: $case_name" ;;
  esac
done

printf 'PASS gpt-webai-slot-pool live smoke cases: %s\n' "${cases[*]}"
printf 'evidence=%s\n' "$evidence_root"
