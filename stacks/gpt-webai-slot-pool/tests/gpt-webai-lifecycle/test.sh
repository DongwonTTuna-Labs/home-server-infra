#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../../.." && pwd)"
supervisor="${GPT_WEBAI_SUPERVISOR:-$repo_root/stacks/gpt-webai-slot-pool/bin/gpt-webai-lifecycle}"
gptpro_wrapper="${GPT_WEBAI_GPTPRO_WRAPPER:-/home/dongwonttuna/.local/bin/gptpro}"
gptxhigh_wrapper="${GPT_WEBAI_GPTXHIGH_WRAPPER:-/home/dongwonttuna/.local/bin/gptxhigh}"
fixture_dir="$script_dir/fixtures"
evidence_root="$(realpath -m -- "$repo_root/.omo/evidence")"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_file() {
  [[ -f "$1" ]] || fail "missing file: $1"
}

assert_dir() {
  [[ -d "$1" ]] || fail "missing dir: $1"
}

assert_eq() {
  [[ "$1" == "$2" ]] || fail "expected [$2], got [$1]"
}

assert_contains() {
  local needle="$1"
  local file="$2"
  grep -F -- "$needle" "$file" >/dev/null || fail "missing [$needle] in $file"
}

assert_not_contains() {
  local needle="$1"
  local file="$2"
  if grep -F -- "$needle" "$file" >/dev/null 2>&1; then
    fail "unexpected [$needle] in $file"
  fi
}

assert_absent_path_fragment() {
  local forbidden="$1"
  local root="$2"
  if grep -R -- "$forbidden" "$root" >/dev/null 2>&1; then
    fail "forbidden path fragment [$forbidden] found under $root"
  fi
}

safe_rm_rf() {
  local target="${1:-}" resolved
  [[ -n "$target" ]] || { printf 'refusing unsafe rm -rf target: empty\n' >&2; return 1; }
  if ! resolved="$(realpath -m -- "$target" 2>/dev/null)"; then
    printf 'refusing unsafe rm -rf target: cannot resolve %s\n' "$target" >&2
    return 1
  fi
  case "$resolved" in
    "$evidence_root"/*) ;;
    *)
      printf 'refusing unsafe rm -rf target outside evidence: %s -> %s\n' "$target" "$resolved" >&2
      return 1
      ;;
  esac
  chmod -R u+w -- "$target" 2>/dev/null || true
  rm -rf -- "$target"
}

run_supervisor() {
  PATH="$fixture_dir/fake-bin:$PATH" \
    GPT_WEBAI_STATE_DIR="$state_dir" \
    GPT_WEBAI_BOOT_ID_FILE="$boot_file" \
    GPT_WEBAI_FAKE_AGBROWSE_LOG="$agbrowse_log" \
    GPT_WEBAI_FAKE_AGBROWSE_ROOT="${fake_agbrowse_root:-$test_root/fake-agbrowse}" \
    GPT_WEBAI_FAKE_MALFORMED_OUTPUT="${fake_malformed_output-}" \
    GPT_WEBAI_AGBROWSE_BIN="$fixture_dir/fake-bin/agbrowse" \
    GPT_WEBAI_SLOT_MODE="${slot_mode-}" \
    GPT_WEBAI_SLOT_COUNT="${slot_count-}" \
    GPT_WEBAI_SLOT_FAKE_LOCAL="${slot_fake_local-}" \
    GPT_WEBAI_FAKE_AUTH_STATE="${fake_auth_state-}" \
    GPT_WEBAI_BROWSER_READY_ATTEMPTS="${ready_attempts:-5}" \
    GPT_WEBAI_BROWSER_READY_DELAY="${ready_delay:-0}" \
    GPT_WEBAI_SEND_SESSION_RETRY_DELAYS="${send_session_retry_delays:-1 3 5 10 15}" \
    GPTPRO_TIMEOUT="${GPTPRO_TIMEOUT-}" \
    GPTXHIGH_TIMEOUT="${GPTXHIGH_TIMEOUT-}" \
    CHROME_BINARY_PATH="${chrome_binary:-$test_root/fake-chrome}" \
    DISPLAY="${display_value-:99}" \
    "$supervisor" "$@"
}

require_supervisor() {
  [[ -x "$supervisor" ]] || fail "missing executable supervisor: $supervisor"
}

write_boot() {
  printf '%s\n' "$1" > "$boot_file"
}

state_core() {
  : "${GPT_WEBAI_TEST_ROOT:?set GPT_WEBAI_TEST_ROOT}"
  test_root="$GPT_WEBAI_TEST_ROOT"
  state_dir="$test_root/state"
  boot_file="$test_root/fake-boot-id"
  agbrowse_log="$test_root/fake-agbrowse.log"

  safe_rm_rf "$test_root"
  mkdir -p "$test_root"
  write_boot boot-a
  require_supervisor

  status_out="$test_root/status.out"
  run_supervisor status > "$status_out"
  assert_contains 'state_dir=' "$status_out"
  assert_contains "tmpdir=$state_dir/tmp" "$status_out"
  assert_contains 'boot_id=boot-a' "$status_out"
  assert_dir "$state_dir"
  assert_dir "$state_dir/tmp"
  assert_dir "$state_dir/holders"
  assert_dir "$state_dir/locks"
  assert_dir "$state_dir/slots/slot-01/state"
  assert_dir "$state_dir/slots/slot-01/attachments"
  assert_dir "$state_dir/slots/slot-10/state"
  assert_dir "$state_dir/slots/slot-10/attachments"
  assert_eq "$(stat -c '%a' "$state_dir/slots/slot-01/state")" 700
  assert_eq "$(stat -c '%a' "$state_dir/slots/slot-01/attachments")" 700
  assert_file "$state_dir/boot-id"
  assert_eq "$(<"$state_dir/boot-id")" boot-a

  constants_out="$test_root/constants.out"
  run_supervisor constants > "$constants_out"
  assert_contains 'EX_OK=0' "$constants_out"
  assert_contains 'EX_USAGE=2' "$constants_out"
  assert_contains 'EX_LOCK=75' "$constants_out"

  retry_delays_out="$test_root/send-retry-delays.out"
  run_supervisor __test send-retry-delays > "$retry_delays_out"
  assert_eq "$(paste -sd, "$retry_delays_out")" "1,3,5,10,15"

  run_supervisor __test atomic-write records/session.json '{"sessionId":"sid-ok"}'
  assert_file "$state_dir/records/session.json"
  assert_eq "$(<"$state_dir/records/session.json")" '{"sessionId":"sid-ok"}'

  lock_subshell_out="$test_root/lock-substitution.out"
  run_supervisor __test lock-substitution > "$lock_subshell_out"
  assert_contains 'lock_survived=1' "$lock_subshell_out"
  assert_contains 'lock_released=1' "$lock_subshell_out"

  if GPT_WEBAI_TEST_INTERRUPT_AFTER_TEMP=1 run_supervisor __test atomic-write records/session.json '{bad-json'; then
    fail 'interrupted atomic write unexpectedly succeeded'
  fi
  assert_eq "$(<"$state_dir/records/session.json")" '{"sessionId":"sid-ok"}'
  [[ -n "$(find "$state_dir/tmp" -type f -name '.atomic-*' -print -quit)" ]] || fail 'interrupted write left no state-dir temp file'
  find "$state_dir/tmp" -type f -name '.atomic-*' -delete

  removed_out="$test_root/removed-lease.out"
  run_supervisor lease create --id live-a --pid "$$" --profile default --session sid-a >"$removed_out" 2>&1
  assert_contains '"reason":"input.usage"' "$removed_out"
  assert_contains 'unknown command: lease' "$removed_out"

  removed_out="$test_root/removed-lock.out"
  run_supervisor lock profile acquire default holder-one >"$removed_out" 2>&1
  assert_contains '"reason":"input.usage"' "$removed_out"
  assert_contains 'unknown command: lock' "$removed_out"

  printf 'id=live-a\npid=%s\nboot_id=boot-a\n' "$$" > "$state_dir/holders/live-a.lease"
  printf 'id=live-b\npid=%s\nboot_id=boot-a\n' "$$" > "$state_dir/holders/live-b.lease"
  assert_file "$state_dir/holders/live-a.lease"
  assert_file "$state_dir/holders/live-b.lease"

  write_boot boot-b
  reboot_prune_out="$test_root/reboot-prune.out"
  run_supervisor status > "$reboot_prune_out"
  assert_contains 'boot_id=boot-b' "$reboot_prune_out"
  [[ ! -e "$state_dir/holders/live-a.lease" ]] || fail 'previous-boot holder survived boot change'
  [[ ! -e "$state_dir/holders/live-b.lease" ]] || fail 'previous-boot holder survived boot change'

  assert_absent_path_fragment "$(printf '/tmp')/gpt-webai" "$state_dir"
  assert_absent_path_fragment "TMPDIR=$(printf '/tmp')" "$script_dir"
  [[ ! -s "$agbrowse_log" ]] || fail 'state-core unexpectedly called fake agbrowse'

  remaining_tmp="$(find "$state_dir/tmp" -mindepth 1 -print -quit)"
  [[ -z "$remaining_tmp" ]] || fail "unexpected temp artifact remains: $remaining_tmp"
  printf 'PASS state-core\n'
  printf 'cleanup: removed interrupted temp files; retained %s as intentional state evidence\n' "$state_dir"
}

write_status_sequence() {
  : > "$fake_agbrowse_root/status-sequence"
  local item
  for item in "$@"; do
    printf '%s\n' "$item" >> "$fake_agbrowse_root/status-sequence"
  done
}

reset_session_case() {
  local name="$1"

  case_root="$test_root/$name"
  state_dir="$case_root/state"
  boot_file="$case_root/fake-boot-id"
  agbrowse_log="$case_root/fake-agbrowse.log"
  fake_agbrowse_root="$case_root/fake-agbrowse"
  chrome_binary="$case_root/fake-chrome"
  display_value=":99"
  ready_attempts=5
  ready_delay=0
  send_session_retry_delays="0 0 0 0 0"
  fake_auth_state=authenticated

  safe_rm_rf "$case_root"
  mkdir -p "$fake_agbrowse_root"
  write_boot boot-a
  printf '#!/usr/bin/env sh\nexit 0\n' > "$chrome_binary"
  chmod +x "$chrome_binary"
  write_status_sequence reachable reachable reachable reachable reachable
}

write_fake_sequence() {
  local name="$1"
  shift
  : > "$fake_agbrowse_root/$name"
  local item
  for item in "$@"; do
    printf '%s\n' "$item" >> "$fake_agbrowse_root/$name"
  done
}

command_count() {
  local needle="$1"
  local count=0
  if [[ -f "$agbrowse_log" ]]; then
    count="$({ grep -F -- "$needle" "$agbrowse_log" || true; } | wc -l | tr -d ' ')"
  fi
  printf '%s' "$count"
}

assert_command_count() {
  local needle="$1" expected="$2"
  assert_eq "$(command_count "$needle")" "$expected"
}

assert_session_record() {
  local sid="$1"
  local found=""
  found="$(grep -R -l -- "sessionId=$sid" "$state_dir/sessions" 2>/dev/null | head -n 1 || true)"
  [[ -n "$found" ]] || fail "missing local session record for $sid"
}

assert_no_session_records() {
  if [[ -d "$state_dir/sessions" ]] && grep -R -q -- 'sessionId=' "$state_dir/sessions" 2>/dev/null; then
    fail "unexpected local session record under $state_dir/sessions"
  fi
}

assert_no_web_ai_stop() {
  assert_command_count 'web-ai stop' 0
}

assert_no_web_ai_send() {
  assert_command_count 'web-ai send' 0
}

assert_log_session_arg() {
  local command="$1" sid="$2"
  assert_contains "web-ai $command" "$agbrowse_log"
  case "$command" in
    sessions\ show|sessions\ resume)
      assert_contains "web-ai $command $sid" "$agbrowse_log"
      ;;
    *)
      assert_contains "--session $sid" "$agbrowse_log"
      ;;
  esac
}

poll_timeout_count() {
  local sid="$1" timeout="$2" count=0
  if [[ -f "$agbrowse_log" ]]; then
    count="$({ grep -F -- "web-ai poll --vendor chatgpt --session $sid --timeout $timeout --json" "$agbrowse_log" || true; } | wc -l | tr -d ' ')"
  fi
  printf '%s' "$count"
}

assert_poll_timeout_count() {
  local sid="$1" timeout="$2" expected="$3" actual
  actual="$(poll_timeout_count "$sid" "$timeout")"
  [[ "$actual" == "$expected" ]] || fail "expected $expected poll call(s) for $sid with --timeout $timeout, got $actual"
}

json_success_envelope() {
  local file="$1" usage_error="${2:-0}"
  python3 - "$file" "$usage_error" <<'PY'
import json
import sys

path = sys.argv[1]
expect_usage_error = sys.argv[2] == "1"
try:
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
except Exception as exc:
    print(f"invalid JSON: {exc}")
    sys.exit(1)

allowed_status = {"done", "running", "recovering", "needs_user_action", "queued"}
errors = []
if data.get("ok") is not True:
    errors.append("ok is not true")
if data.get("hardFailure") is not False:
    errors.append("hardFailure is not false")
if data.get("networkDisconnected") is not False:
    errors.append("networkDisconnected is not false")
if data.get("status") not in allowed_status:
    errors.append("status is not a recovery/success state")
if expect_usage_error and data.get("usageError") is not True:
    errors.append("usageError is not true")
if errors:
    print("; ".join(errors))
    sys.exit(1)
PY
}

json_hard_network_envelope() {
  local file="$1"
  python3 - "$file" <<'PY'
import json
import sys

path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
except Exception as exc:
    print(f"invalid JSON: {exc}")
    sys.exit(1)

errors = []
if data.get("ok") is not True:
    errors.append("ok is not true")
if data.get("hardFailure") is not False:
    errors.append("hardFailure is not false")
if data.get("networkDisconnected") is not True:
    errors.append("networkDisconnected is not true")
if data.get("reason") != "network.disconnected":
    errors.append("reason is not network.disconnected")
if data.get("status") != "recovering":
    errors.append("status is not recovering")
if not data.get("message"):
    errors.append("message is empty")
if not data.get("nextCommand"):
    errors.append("nextCommand is empty")
if errors:
    print("; ".join(errors))
    sys.exit(1)
PY
}

json_string_field_equals() {
  local file="$1" field="$2" expected="$3"
  python3 - "$file" "$field" "$expected" <<'PY'
import json
import sys

path, field, expected = sys.argv[1:]
with open(path, encoding="utf-8") as handle:
    data = json.load(handle)
actual = data.get(field)
if actual != expected:
    print(f"{field}: expected {expected!r}, got {actual!r}")
    sys.exit(1)
PY
}

json_field_absent() {
  local file="$1" field="$2"
  python3 - "$file" "$field" <<'PY'
import json
import sys

path, field = sys.argv[1:]
with open(path, encoding="utf-8") as handle:
    data = json.load(handle)
if field in data:
    print(f"{field}: expected absent, got {data[field]!r}")
    sys.exit(1)
PY
}

assert_success_envelope_fields() {
  local file="$1" reason="$2" status="$3"
  json_success_envelope "$file" 0
  json_string_field_equals "$file" reason "$reason"
  json_string_field_equals "$file" status "$status"
}

assert_resume_command() {
  local file="$1" kind="$2" sid="$3"
  json_string_field_equals "$file" resumeCommand "gpt-webai-lifecycle resume --kind $kind --session $sid"
}

request_fingerprint_fixture() {
  local kind="$1" model="$2" effort="$3" prompt="$4"
  python3 - "$kind" "$model" "$effort" "$prompt" <<'PY'
import hashlib
import json
import sys

kind, model, effort, prompt = sys.argv[1:]
payload = {
    "effort": effort,
    "kind": kind,
    "model": model,
    "promptSha256": hashlib.sha256(prompt.encode("utf-8")).hexdigest(),
}
encoded = json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
print(hashlib.sha256(encoded).hexdigest())
PY
}

assert_session_record_contains() {
  local sid="$1" needle="$2" found=""
  found="$(grep -R -l -- "sessionId=$sid" "$state_dir/sessions" 2>/dev/null | head -n 1 || true)"
  [[ -n "$found" ]] || fail "missing local session record for $sid"
  assert_contains "$needle" "$found"
}

event_log_file() {
  printf '%s/events/lifecycle.jsonl' "$state_dir"
}

assert_event_path_safe() {
  local file
  file="$(event_log_file)"
  case "$file" in
    "$state_dir"/events/lifecycle.jsonl) ;;
    *) fail "event log outside state dir: $file" ;;
  esac
  case "$file" in
    /tmp/*) fail "event log under /tmp: $file" ;;
  esac
}

assert_event_reason() {
  local reason="$1" file
  file="$(event_log_file)"
  assert_event_path_safe
  assert_file "$file"
  python3 - "$file" "$reason" <<'PY'
import json
import sys

path, expected = sys.argv[1:]
found = False
with open(path, encoding="utf-8") as handle:
    for line_number, line in enumerate(handle, 1):
        data = json.loads(line)
        for key in ("timestamp", "event", "reason", "status"):
            if not data.get(key):
                print(f"event line {line_number} missing {key}")
                sys.exit(1)
        for unsafe_key in ("message", "prompt", "output", "raw", "env", "cookie"):
            if unsafe_key in data:
                print(f"event line {line_number} logged unsafe key {unsafe_key}")
                sys.exit(1)
        if data.get("reason") == expected:
            found = True
if not found:
    print(f"missing event reason {expected!r} in {path}")
    sys.exit(1)
PY
}

assert_event_session_reason() {
  local reason="$1" sid="$2" file
  file="$(event_log_file)"
  assert_file "$file"
  python3 - "$file" "$reason" "$sid" <<'PY'
import json
import sys

path, expected_reason, expected_sid = sys.argv[1:]
with open(path, encoding="utf-8") as handle:
    for line in handle:
        data = json.loads(line)
        if data.get("reason") == expected_reason and data.get("sessionId") == expected_sid:
            sys.exit(0)
print(f"missing event reason/session {expected_reason!r}/{expected_sid!r} in {path}")
sys.exit(1)
PY
}

assert_events_absent() {
  local needle="$1" root="$2"
  [[ -n "$needle" ]] || return 0
  if find "$root" -path '*/events/*' -type f -exec grep -F -l -- "$needle" {} + 2>/dev/null | grep -q .; then
    fail "event files leaked [$needle] under $root"
  fi
}

assert_observability_static_scan() {
  python3 - "$supervisor" "$script_dir/test.sh" "$fixture_dir/fake-bin/agbrowse" <<'PY'
import pathlib
import sys

patterns = ["/tmp/" + "gpt-webai", "mktemp" + " -t"]
for path_text in sys.argv[1:]:
    path = pathlib.Path(path_text)
    text = path.read_text(encoding="utf-8")
    for pattern in patterns:
        if pattern in text:
            print(f"forbidden static string {pattern!r} in {path}")
            sys.exit(1)
PY
}

run_nonnetwork_command() {
  local out="$1" err="$2" status
  shift 2
  set +e
  "$@" > "$out" 2> "$err"
  status=$?
  set -e
  printf '%s' "$status"
}

write_lock_holder() {
  local name="$1" holder="$2" pid="$3" boot="$4"
  mkdir -p "$state_dir/locks/$name.lock"
  cat > "$state_dir/locks/$name.lock/holder" <<EOF
holder=$holder
boot_id=$boot
pid=$pid
EOF
}

reset_slot_case() {
  reset_session_case "$1"
  slot_mode=fake
  slot_count=2
}

slot_broker() {
  : "${GPT_WEBAI_TEST_ROOT:?set GPT_WEBAI_TEST_ROOT}"
  test_root="$GPT_WEBAI_TEST_ROOT"
  safe_rm_rf "$test_root"
  mkdir -p "$test_root"
  require_supervisor

  reset_slot_case slot-attachment-redaction
  attachment="$case_root/original secret basename.txt"
  printf 'slot attachment payload\n' > "$attachment"
  write_fake_sequence send-sequence sid:sid-slot-redact
  write_fake_sequence poll-sequence done:redact
  run_supervisor run --kind pro --file "$attachment" --prompt 'slot redact' > "$case_root/out" 2> "$case_root/err"
  assert_contains '"sessionId":"sid-slot-redact"' "$case_root/out"
  assert_contains 'BROWSER_AGENT_HOME='"$state_dir"'/slots/slot-01/state CDP_PORT=9223' "$fake_agbrowse_root/browser-agent-home-calls"
  assert_contains '--file /broker-attachments/' "$agbrowse_log"
  assert_contains '/files/001-' "$agbrowse_log"
  assert_not_contains 'original secret basename' "$agbrowse_log"
  assert_not_contains "$attachment" "$agbrowse_log"
  prompt_file="$(find "$state_dir/requests" -name prompt.txt -print -quit)"
  assert_file "$prompt_file"
  assert_contains 'ATTACHMENT_ACCESS_GATE:' "$prompt_file"
  assert_contains 'ATTACHMENT_MISSING' "$prompt_file"
  assert_contains '001-' "$prompt_file"
  assert_not_contains "$attachment" "$prompt_file"
  assert_session_record_contains sid-slot-redact 'slotId=slot-01'

  reset_slot_case slot-attachment-missing-envelope
  attachment_canary="$case_root/canary.md"
  printf 'ATTACHMENT_CANARY_FAKE\n' > "$attachment_canary"
  write_fake_sequence send-sequence sid:sid-attachment-missing
  write_fake_sequence poll-sequence done-attachment-missing
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --file "$attachment_canary" --prompt 'read canary')"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" provider.attachment_unavailable recovering
  json_field_absent "$case_root/out" resumeCommand
  assert_command_count 'web-ai send' 1
  assert_command_count 'web-ai poll' 1
  assert_session_record_contains sid-attachment-missing 'status=attachment_missing'
  assert_session_record_contains sid-attachment-missing 'attachmentCount=1'
  assert_no_web_ai_stop

  reset_slot_case slot-attachment-missing-resume
  mkdir -p "$state_dir/sessions"
  cat > "$state_dir/sessions/slot-attached-running.record" <<'EOF'
sessionId=sid-attachment-resume
kind=pro
model=pro
effort=extended
fingerprint=slot-attached-running
slotId=slot-01
slotContainer=gpt-webai-slot-01
attachmentCount=1
status=running
EOF
  write_fake_sequence resume-sequence resumed
  write_fake_sequence poll-sequence done-attachment-missing
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor resume --kind pro --session sid-attachment-resume)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" provider.attachment_unavailable recovering
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai sessions resume' 1
  assert_command_count 'web-ai poll' 1
  assert_session_record_contains sid-attachment-resume 'status=attachment_missing'
  assert_session_record_contains sid-attachment-resume 'attachmentCount=1'
  assert_no_web_ai_stop

  reset_slot_case slot-second-when-first-busy
  write_lock_holder runtime-slot-slot-01 runtime "$$" boot-a
  write_fake_sequence send-sequence sid:sid-slot-two
  write_fake_sequence poll-sequence done:slot-two
  run_supervisor run --kind pro --prompt 'slot two' > "$case_root/out" 2> "$case_root/err"
  assert_contains '"sessionId":"sid-slot-two"' "$case_root/out"
  assert_contains 'BROWSER_AGENT_HOME='"$state_dir"'/slots/slot-02/state CDP_PORT=9224' "$fake_agbrowse_root/browser-agent-home-calls"
  assert_session_record_contains sid-slot-two 'slotId=slot-02'

  reset_slot_case slot-missing-holder-lock-not-stolen
  mkdir -p "$state_dir/locks/runtime-slot-slot-01.lock"
  write_fake_sequence send-sequence sid:sid-slot-missing-holder
  write_fake_sequence poll-sequence done:missing-holder
  run_supervisor run --kind pro --prompt 'slot missing holder' > "$case_root/out" 2> "$case_root/err"
  assert_contains '"sessionId":"sid-slot-missing-holder"' "$case_root/out"
  assert_contains 'BROWSER_AGENT_HOME='"$state_dir"'/slots/slot-02/state CDP_PORT=9224' "$fake_agbrowse_root/browser-agent-home-calls"
  assert_session_record_contains sid-slot-missing-holder 'slotId=slot-02'

  local blocked_status
  for blocked_status in repairing warming reseed_login degraded; do
    reset_slot_case "slot-skip-$blocked_status"
    mkdir -p "$state_dir/slots"
    printf 'status=%s\nslotId=slot-01\n' "$blocked_status" > "$state_dir/slots/slot-01.state"
    write_status_sequence reachable
    write_fake_sequence send-sequence "sid:sid-slot-$blocked_status"
    write_fake_sequence poll-sequence "done:skip-$blocked_status"
    run_supervisor run --kind pro --prompt "skip $blocked_status slot" > "$case_root/out" 2> "$case_root/err"
    assert_contains "\"sessionId\":\"sid-slot-$blocked_status\"" "$case_root/out"
    assert_contains 'BROWSER_AGENT_HOME='"$state_dir"'/slots/slot-02/state CDP_PORT=9224' "$fake_agbrowse_root/browser-agent-home-calls"
    assert_session_record_contains "sid-slot-$blocked_status" 'slotId=slot-02'
  done

  reset_slot_case slot-ensure-routes-to-slot
  write_status_sequence unreachable
  exit_code="$(GPT_WEBAI_SLOT_REPAIR_MAX_ATTEMPTS=1 run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor browser ensure --slot slot-01)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" browser.cdp_unreachable recovering
  json_string_field_equals "$case_root/out" nextCommand 'gpt-webai-lifecycle browser ensure --slot slot-01'
  json_field_absent "$case_root/out" resumeCommand
  assert_file "$state_dir/slots/slot-01.repair"
  assert_contains 'attempts=1' "$state_dir/slots/slot-01.repair"
  assert_contains 'status=degraded' "$state_dir/slots/slot-01.repair"
  assert_contains 'status=degraded' "$state_dir/slots/slot-01.state"

  reset_slot_case slot-repair-backoff
  write_status_sequence unreachable unreachable
  exit_code="$(GPT_WEBAI_SLOT_REPAIR_MAX_ATTEMPTS=3 GPT_WEBAI_SLOT_REPAIR_BACKOFF_SECONDS=3600 run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor browser ensure --slot slot-01)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" browser.cdp_unreachable recovering
  assert_contains 'attempts=1' "$state_dir/slots/slot-01.repair"
  assert_contains 'status=repairing' "$state_dir/slots/slot-01.repair"
  exit_code="$(GPT_WEBAI_SLOT_REPAIR_MAX_ATTEMPTS=3 GPT_WEBAI_SLOT_REPAIR_BACKOFF_SECONDS=3600 run_nonnetwork_command "$case_root/backoff.out" "$case_root/backoff.err" run_supervisor browser ensure --slot slot-01)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/backoff.out" browser.cdp_unreachable recovering
  json_field_absent "$case_root/backoff.out" resumeCommand
  assert_contains 'attempts=1' "$state_dir/slots/slot-01.repair"
  run_supervisor status > "$case_root/status.out" 2> "$case_root/status.err"
  assert_contains 'slot_01_status=repairing' "$case_root/status.out"
  assert_contains 'slot_01_repair_attempts=1' "$case_root/status.out"
  assert_contains 'slot_01_repair_max_attempts=3' "$case_root/status.out"
  assert_contains 'slot_01_next_retry_at=' "$case_root/status.out"

  reset_slot_case slot-browser-ensure-requires-slot
  write_status_sequence reachable
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor browser ensure)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.slot_required recovering
  assert_command_count 'web-ai status' 0
  assert_command_count 'agbrowse start' 0

  reset_slot_case slot-browser-ensure-login-required
  fake_auth_state=login_required
  write_status_sequence web-ai-ready
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor browser ensure --slot slot-01)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" auth.needs_login needs_user_action
  assert_contains 'status=reseed_login' "$state_dir/slots/slot-01.state"
  assert_contains 'reason=auth.needs_login' "$state_dir/slots/slot-01.state"
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai poll' 0

  reset_slot_case slot-run-login-required-no-send
  fake_auth_state=login_required
  write_status_sequence web-ai-ready
  write_fake_sequence send-sequence fail-if-called
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --prompt 'must not send while logged out')"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" auth.needs_login needs_user_action
  assert_contains 'status=reseed_login' "$state_dir/slots/slot-01.state"
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai poll' 0

  reset_slot_case slot-pool-queued
  write_lock_holder runtime-slot-slot-01 runtime "$$" boot-a
  write_lock_holder runtime-slot-slot-02 runtime "$$" boot-a
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --prompt 'queued slot')"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.pool_busy queued
  queued_fp="$(request_fingerprint_fixture pro pro extended 'queued slot')"
  json_string_field_equals "$case_root/out" nextCommand "gpt-webai-lifecycle queue resume --request $queued_fp"
  assert_command_count 'web-ai send' 0
  rm -rf "$state_dir/locks/runtime-slot-slot-01.lock" "$state_dir/locks/runtime-slot-slot-02.lock"
  write_fake_sequence send-sequence sid:sid-queued-resume
  write_fake_sequence poll-sequence done:queued-resume
  run_supervisor queue resume --request "$queued_fp" > "$case_root/resume.out" 2> "$case_root/resume.err"
  assert_contains '"sessionId":"sid-queued-resume"' "$case_root/resume.out"
  assert_command_count 'web-ai send' 1

  reset_slot_case slot-resume-lock-busy
  mkdir -p "$state_dir/sessions"
  cat > "$state_dir/sessions/slot-existing.record" <<'EOF'
sessionId=sid-slot-resume
kind=pro
model=pro
effort=extended
fingerprint=slot-existing
slotId=slot-01
slotContainer=gpt-webai-slot-01
status=done
EOF
  write_lock_holder runtime-slot-slot-01 runtime "$$" boot-a
  write_fake_sequence poll-sequence done:must-not-poll
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor resume --kind pro --session sid-slot-resume)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" session.running running
  assert_resume_command "$case_root/out" pro sid-slot-resume
  assert_command_count 'web-ai sessions resume' 0
  assert_command_count 'web-ai poll' 0

  reset_slot_case slot-browser-ensure-lock-busy
  write_lock_holder runtime-slot-slot-01 runtime "$$" boot-a
  write_status_sequence reachable
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor browser ensure --slot slot-01)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.busy recovering
  assert_command_count 'web-ai status' 0
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai poll' 0

  reset_slot_case slot-browser-ensure-active-record-busy
  mkdir -p "$state_dir/sessions"
  cat > "$state_dir/sessions/slot-active.record" <<'EOF'
sessionId=sid-slot-active
kind=pro
model=pro
effort=extended
fingerprint=slot-active
slotId=slot-01
slotContainer=gpt-webai-slot-01
status=running
EOF
  write_status_sequence reachable
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor browser ensure --slot slot-01)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.busy recovering
  assert_command_count 'web-ai status' 0
  run_supervisor status > "$case_root/status.out" 2> "$case_root/status.err"
  assert_contains 'slot_01_status=busy' "$case_root/status.out"

  reset_session_case slot-backed-pool-disabled-resume
  slot_mode=off
  mkdir -p "$state_dir/sessions"
  cat > "$state_dir/sessions/slot-disabled-resume.record" <<'EOF'
sessionId=sid-slot-disabled-resume
kind=pro
model=pro
effort=extended
fingerprint=slot-disabled-resume
slotId=slot-01
slotContainer=gpt-webai-slot-01
status=done
EOF
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor resume --kind pro --session sid-slot-disabled-resume)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.pool_unavailable recovering
  assert_resume_command "$case_root/out" pro sid-slot-disabled-resume
  assert_command_count 'web-ai sessions resume' 0
  assert_command_count 'web-ai poll' 0

  reset_session_case slot-backed-pool-disabled-run
  slot_mode=off
  mkdir -p "$state_dir/sessions"
  disabled_prompt='slot backed disabled run'
  disabled_fp="$(request_fingerprint_fixture pro pro extended "$disabled_prompt")"
  cat > "$state_dir/sessions/$disabled_fp.record" <<EOF
sessionId=sid-slot-disabled-run
kind=pro
model=pro
effort=extended
fingerprint=$disabled_fp
slotId=slot-01
slotContainer=gpt-webai-slot-01
status=done
EOF
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --prompt "$disabled_prompt")"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.pool_unavailable recovering
  assert_resume_command "$case_root/out" pro sid-slot-disabled-run
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai poll' 0

  reset_session_case slot-backed-pool-disabled-queue
  slot_mode=off
  mkdir -p "$state_dir/sessions"
  cat > "$state_dir/sessions/slot-disabled-queue.record" <<'EOF'
sessionId=sid-slot-disabled-queue
kind=pro
model=pro
effort=extended
fingerprint=slot-disabled-queue
slotId=slot-01
slotContainer=gpt-webai-slot-01
status=queued
EOF
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor queue resume --request slot-disabled-queue)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.pool_unavailable recovering
  assert_resume_command "$case_root/out" pro sid-slot-disabled-queue
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai poll' 0

  reset_session_case slot-backed-pool-disabled-queue-sendless
  slot_mode=off
  mkdir -p "$state_dir/sessions"
  cat > "$state_dir/sessions/slot-disabled-sendless.record" <<'EOF'
sessionId=
kind=pro
model=pro
effort=extended
fingerprint=slot-disabled-sendless
slotId=slot-01
slotContainer=gpt-webai-slot-01
status=queued
EOF
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor queue resume --request slot-disabled-sendless)"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" slot.pool_unavailable recovering
  json_field_absent "$case_root/out" resumeCommand
  assert_command_count 'web-ai send' 0
  assert_command_count 'web-ai poll' 0

  reset_slot_case auth-seed-attachment-rejected
  mkdir -p "$state_dir/auth-seed"
  printf 'cookie-like-secret\n' > "$state_dir/auth-seed/seed.txt"
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --file "$state_dir/auth-seed/seed.txt" --prompt 'must reject seed')"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" input.invalid_file needs_user_action
  assert_command_count 'web-ai send' 0

  reset_slot_case slot-send-no-session-retries
  unknown_fp="$(request_fingerprint_fixture pro pro extended 'slot no sid retry')"
  write_fake_sequence send-sequence crash-no-sid provider-missing-fields sid:sid-after-retry
  write_fake_sequence poll-sequence done:no-session-recovered
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --prompt 'slot no sid retry')"
  assert_eq "$exit_code" 0
  assert_contains '"sessionId":"sid-after-retry"' "$case_root/out"
  assert_command_count 'web-ai send' 3
  assert_command_count 'web-ai poll' 1
  assert_contains 'status=done' "$state_dir/sessions/$unknown_fp.record"
  assert_session_record_contains sid-after-retry 'slotId=slot-01'
  assert_event_reason send.unknown_session
  run_supervisor status > "$case_root/status.out" 2> "$case_root/status.err"
  assert_contains 'slot_01_status=ready' "$case_root/status.out"

  reset_slot_case slot-send-no-session-retry-exhausted
  send_session_retry_delays="0 0"
  unknown_fp="$(request_fingerprint_fixture pro pro extended 'slot no sid exhausted')"
  write_fake_sequence send-sequence crash-no-sid provider-missing-fields crash-no-sid sid:sid-exhausted-rerun
  exit_code="$(run_nonnetwork_command "$case_root/out" "$case_root/err" run_supervisor run --kind pro --prompt 'slot no sid exhausted')"
  assert_eq "$exit_code" 0
  assert_success_envelope_fields "$case_root/out" send.unknown_session recovering
  json_field_absent "$case_root/out" resumeCommand
  assert_command_count 'web-ai send' 3
  assert_contains 'status=send_unknown_session' "$state_dir/sessions/$unknown_fp.record"
  run_supervisor status > "$case_root/status.out" 2> "$case_root/status.err"
  assert_contains 'slot_01_status=ready' "$case_root/status.out"
  write_fake_sequence poll-sequence done:exhausted-rerun
  exit_code="$(run_nonnetwork_command "$case_root/reuse.out" "$case_root/reuse.err" run_supervisor run --kind pro --prompt 'slot no sid exhausted')"
  assert_eq "$exit_code" 0
  assert_contains '"sessionId":"sid-exhausted-rerun"' "$case_root/reuse.out"
  assert_command_count 'web-ai send' 4
  assert_command_count 'web-ai poll' 1

  printf 'PASS slot-broker\n'
  printf 'cleanup: fake slot broker cases retained under %s; no real Docker/Chrome/ChatGPT used\n' "$test_root"
}

write_fake_wrapper_bins() {
  fake_wrapper_bin="$test_root/fake-wrapper-bin"
  supervisor_log="$test_root/fake-supervisor.log"
  agbrowse_log="$test_root/forbidden-agbrowse.log"
  mkdir -p "$fake_wrapper_bin"

  cat > "$fake_wrapper_bin/gpt-webai-lifecycle" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

log="${GPT_WEBAI_FAKE_SUPERVISOR_LOG:?set GPT_WEBAI_FAKE_SUPERVISOR_LOG}"
kind=""
prompt=""
files=()
display_prompt=""

printf 'gpt-webai-lifecycle' >> "$log"
for arg in "$@"; do
  printf ' %q' "$arg" >> "$log"
done
printf ' timeout_pro=%s timeout_xhigh=%s\n' "${GPTPRO_TIMEOUT:-}" "${GPTXHIGH_TIMEOUT:-}" >> "$log"

[[ "${1:-}" == run ]] || { printf 'expected run, got %s\n' "${1:-}" >&2; exit 2; }
shift
while [[ $# -gt 0 ]]; do
  case "$1" in
    --kind) kind="${2:-}"; shift 2 ;;
    --prompt) prompt="${2:-}"; shift 2 ;;
    --file) files+=("${2:-}"); shift 2 ;;
    *) printf 'unknown fake supervisor arg: %s\n' "$1" >&2; exit 2 ;;
  esac
done
[[ -n "$kind" && -n "$prompt" ]] || { printf 'missing kind or prompt\n' >&2; exit 2; }
display_prompt="$prompt"
marker=$'\n\nUSER TASK:\n'
if [[ "$display_prompt" == *"$marker"* ]]; then
  display_prompt="${display_prompt##*"$marker"}"
fi
python3 - "$kind" "$display_prompt" "${#files[@]}" <<'PY'
import json
import sys

print(json.dumps(
    {"fake": True, "kind": sys.argv[1], "prompt": sys.argv[2], "fileCount": int(sys.argv[3])},
    ensure_ascii=False,
    separators=(",", ":"),
))
PY
EOF
  chmod +x "$fake_wrapper_bin/gpt-webai-lifecycle"

  cat > "$fake_wrapper_bin/agbrowse" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

log="${GPT_WEBAI_FORBIDDEN_AGBROWSE_LOG:?set GPT_WEBAI_FORBIDDEN_AGBROWSE_LOG}"
printf 'agbrowse' >> "$log"
for arg in "$@"; do
  printf ' %q' "$arg" >> "$log"
done
printf '\n' >> "$log"
printf 'forbidden agbrowse call from wrapper\n' >&2
exit 71
EOF
  chmod +x "$fake_wrapper_bin/agbrowse"
}

run_wrapper() {
  local wrapper="$1"
  shift
  PATH="$fake_wrapper_bin:$PATH" \
    GPT_WEBAI_FAKE_SUPERVISOR_LOG="$supervisor_log" \
    GPT_WEBAI_FORBIDDEN_AGBROWSE_LOG="$agbrowse_log" \
    "$wrapper" "$@"
}

run_wrapper_stdin() {
  local wrapper="$1" input="$2"
  shift 2
  PATH="$fake_wrapper_bin:$PATH" \
    GPT_WEBAI_FAKE_SUPERVISOR_LOG="$supervisor_log" \
    GPT_WEBAI_FORBIDDEN_AGBROWSE_LOG="$agbrowse_log" \
    "$wrapper" "$@" <<< "$input"
}

supervisor_call_count() {
  local count=0
  if [[ -f "$supervisor_log" ]]; then
    count="$(wc -l < "$supervisor_log" | tr -d ' ')"
  fi
  printf '%s' "$count"
}

assert_supervisor_call_count() {
  local expected="$1"
  assert_eq "$(supervisor_call_count)" "$expected"
}

assert_wrapper_usage_envelope() {
  local file="$1"
  json_success_envelope "$file" 1
  json_string_field_equals "$file" reason input.empty_prompt
  json_string_field_equals "$file" status needs_user_action
  json_string_field_equals "$file" nextCommand 'gpt-webai-lifecycle --help'
}

wrappers() {
  : "${GPT_WEBAI_TEST_ROOT:?set GPT_WEBAI_TEST_ROOT}"
  test_root="$GPT_WEBAI_TEST_ROOT"
  safe_rm_rf "$test_root"
  mkdir -p "$test_root"
  [[ -x "$gptpro_wrapper" ]] || fail "missing executable gptpro wrapper: $gptpro_wrapper"
  [[ -x "$gptxhigh_wrapper" ]] || fail "missing executable gptxhigh wrapper: $gptxhigh_wrapper"
  write_fake_wrapper_bins

  run_wrapper "$gptpro_wrapper" hello pro > "$test_root/gptpro-argv.out" 2> "$test_root/gptpro-argv.err"
  assert_contains '"kind":"pro"' "$test_root/gptpro-argv.out"
  assert_contains '"prompt":"hello pro"' "$test_root/gptpro-argv.out"
  assert_contains 'gpt-webai-lifecycle run --kind pro' "$supervisor_log"
  assert_contains 'timeout_pro=10800' "$supervisor_log"

  wrapper_file_one="$test_root/wrapper-one.txt"
  wrapper_file_two="$test_root/wrapper-two.txt"
  printf 'one\n' > "$wrapper_file_one"
  printf 'two\n' > "$wrapper_file_two"
  run_wrapper "$gptpro_wrapper" --file "$wrapper_file_one" --file="$wrapper_file_two" review attachments > "$test_root/gptpro-files.out" 2> "$test_root/gptpro-files.err"
  assert_contains '"kind":"pro"' "$test_root/gptpro-files.out"
  assert_contains '"prompt":"review attachments"' "$test_root/gptpro-files.out"
  assert_contains '"fileCount":2' "$test_root/gptpro-files.out"
  assert_contains "--file $wrapper_file_one --file $wrapper_file_two" "$supervisor_log"

  run_wrapper_stdin "$gptpro_wrapper" 'stdin pro prompt' > "$test_root/gptpro-stdin.out" 2> "$test_root/gptpro-stdin.err"
  assert_contains '"kind":"pro"' "$test_root/gptpro-stdin.out"
  assert_contains '"prompt":"stdin pro prompt"' "$test_root/gptpro-stdin.out"
  assert_contains 'timeout_pro=10800' "$supervisor_log"

  run_wrapper_stdin "$gptpro_wrapper" 'stdin file prompt' --file "$wrapper_file_one" > "$test_root/gptpro-file-stdin.out" 2> "$test_root/gptpro-file-stdin.err"
  assert_contains '"kind":"pro"' "$test_root/gptpro-file-stdin.out"
  assert_contains '"prompt":"stdin file prompt"' "$test_root/gptpro-file-stdin.out"
  assert_contains '"fileCount":1' "$test_root/gptpro-file-stdin.out"

  run_wrapper "$gptxhigh_wrapper" hello xhigh > "$test_root/gptxhigh-argv.out" 2> "$test_root/gptxhigh-argv.err"
  assert_contains '"kind":"xhigh"' "$test_root/gptxhigh-argv.out"
  assert_contains '"prompt":"hello xhigh"' "$test_root/gptxhigh-argv.out"
  assert_contains 'gpt-webai-lifecycle run --kind xhigh' "$supervisor_log"
  assert_contains 'timeout_xhigh=300' "$supervisor_log"

  run_wrapper_stdin "$gptxhigh_wrapper" 'stdin xhigh prompt' > "$test_root/gptxhigh-stdin.out" 2> "$test_root/gptxhigh-stdin.err"
  assert_contains '"kind":"xhigh"' "$test_root/gptxhigh-stdin.out"
  assert_contains '"prompt":"stdin xhigh prompt"' "$test_root/gptxhigh-stdin.out"
  assert_contains 'timeout_xhigh=300' "$supervisor_log"
  assert_supervisor_call_count 6
  [[ ! -s "$agbrowse_log" ]] || fail 'wrapper called forbidden agbrowse directly'

  exit_code="$(run_nonnetwork_command "$test_root/gptpro-empty-argv.out" "$test_root/gptpro-empty-argv.err" run_wrapper "$gptpro_wrapper" "")"
  assert_eq "$exit_code" 0
  assert_wrapper_usage_envelope "$test_root/gptpro-empty-argv.out"
  assert_supervisor_call_count 6

  exit_code="$(run_nonnetwork_command "$test_root/gptpro-empty-stdin.out" "$test_root/gptpro-empty-stdin.err" run_wrapper_stdin "$gptpro_wrapper" '   ')"
  assert_eq "$exit_code" 0
  assert_wrapper_usage_envelope "$test_root/gptpro-empty-stdin.out"
  assert_supervisor_call_count 6

  exit_code="$(run_nonnetwork_command "$test_root/gptxhigh-empty-argv.out" "$test_root/gptxhigh-empty-argv.err" run_wrapper "$gptxhigh_wrapper" "")"
  assert_eq "$exit_code" 0
  assert_wrapper_usage_envelope "$test_root/gptxhigh-empty-argv.out"
  assert_supervisor_call_count 6

  exit_code="$(run_nonnetwork_command "$test_root/gptxhigh-empty-stdin.out" "$test_root/gptxhigh-empty-stdin.err" run_wrapper_stdin "$gptxhigh_wrapper" '   ')"
  assert_eq "$exit_code" 0
  assert_wrapper_usage_envelope "$test_root/gptxhigh-empty-stdin.out"
  assert_supervisor_call_count 6
  [[ ! -s "$agbrowse_log" ]] || fail 'empty prompt called forbidden agbrowse directly'

  printf 'PASS wrappers\n'
  printf 'cleanup: fake supervisor and wrapper outputs retained under %s; no real agbrowse/ChatGPT/Chrome/Xvfb used\n' "$test_root"
}

safe_root() {
  : "${GPT_WEBAI_TEST_ROOT:?set GPT_WEBAI_TEST_ROOT}"
  test_root="$GPT_WEBAI_TEST_ROOT"
  safe_rm_rf "$test_root"
  mkdir -p "$test_root"

  local allowed="$test_root/safe-root/allowed" err index unsafe
  mkdir -p "$allowed"
  printf 'delete me\n' > "$allowed/file"
  safe_rm_rf "$allowed"
  [[ ! -e "$allowed" ]] || fail "safe evidence-local cleanup did not remove: $allowed"

  index=0
  for unsafe in "" / "$HOME" "$repo_root" "$(dirname "$repo_root")" "$repo_root/.omo/evidence/../not-evidence" "$repo_root/.omo/not-evidence"; do
    index=$((index + 1))
    err="$test_root/safe-root-unsafe-$index.err"
    if safe_rm_rf "$unsafe" 2> "$err"; then
      fail "unsafe root accepted: $unsafe"
    fi
    assert_contains 'refusing unsafe rm -rf target' "$err"
  done

  printf 'PASS safe-root\n'
}

case "${1:-}" in
  safe-root)
    safe_root
    ;;
  state-core)
    state_core
    ;;
  slot-broker)
    slot_broker
    ;;
  wrappers)
    wrappers
    ;;
  all)
    safe_root
    state_core
    slot_broker
    wrappers
    ;;
  *)
    printf 'Usage: %s safe-root|state-core|slot-broker|wrappers|all\n' "$0" >&2
    exit 2
    ;;
esac
