capture_preflight() {
  local case_dir="$1" label="$2"
  mkdir -p "$case_dir/preflight"
  write_command_file "$case_dir/preflight/command.txt" "$lifecycle" preflight --json --docker-slot-provider --run-id "$label"
  "$lifecycle" preflight --json --docker-slot-provider --run-id "$label" >"$case_dir/preflight/out.json" 2>"$case_dir/preflight/err"
  printf '%s\n' "$?" >"$case_dir/preflight/rc"
  if [[ "$(json_get "$case_dir/preflight/out.json" ok)" != true ]]; then
    if [[ "$(json_get "$case_dir/preflight/out.json" reason)" == slot.unavailable ]]; then
      "$lifecycle" status --json >"$case_dir/preflight/status-after-unavailable.json" 2>"$case_dir/preflight/status-after-unavailable.err"
      printf '%s\n' "$?" >"$case_dir/preflight/status-after-unavailable.rc"
      node -e '
        const fs = require("fs");
        const status = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
        const startable = status.slots.some((slot) =>
          slot.allocatable === true &&
          (slot.status === "standby" || slot.docker_status === "exited")
        );
        if (!startable) process.exit(1);
      ' "$case_dir/preflight/status-after-unavailable.json" \
        || fail "preflight unavailable and no startable standby slot for $label"
      printf '%s\n' 'preflight unavailable because no provider-ready runtime is already running; run path must start a standby slot and capture pre-send diagnostics' >"$case_dir/preflight/startable-standby-note.txt"
      return 0
    fi
    fail "preflight not ok for $label"
  fi
  [[ "$(json_get "$case_dir/preflight/out.json" status)" == ready ]] || fail "preflight not ready for $label"
}

record_status() {
  local case_dir="$1" label="$2"
  "$lifecycle" status --json >"$case_dir/status.$label.json" 2>"$case_dir/status.$label.err"
  printf '%s\n' "$?" >"$case_dir/status.$label.rc"
}

validate_run_result() {
  local case_dir="$1" result="$2" token="$3" min_artifacts="${4:-0}"
  local ok status session_id url slot group art_dir answer_file answer_json
  ok="$(json_get "$result" ok)"
  status="$(json_get "$result" status)"
  session_id="$(json_get "$result" sessionId)"
  url="$(json_get "$result" conversationUrl)"
  slot="$(json_get "$result" slotId)"
  group="$(json_get "$result" accountGroup)"
  [[ "$ok" == true ]] || fail "run not ok: $result"
  [[ "$status" == done ]] || fail "run status is $status: $result"
  [[ "$session_id" =~ ^[0-9a-f-]{36}$ ]] || fail "bad sessionId: $session_id"
  [[ "$url" == "https://chatgpt.com/c/$session_id" ]] || fail "bad conversationUrl: $url"
  [[ "$slot" =~ ^slot-[0-9]{2}$ ]] || fail "bad slotId: $slot"
  [[ "$group" =~ ^group-0[12]$ ]] || fail "bad accountGroup: $group"
  json_number_ge "$result" artifacts "$min_artifacts" || fail "expected at least $min_artifacts artifacts in $result"

  art_dir="$(artifact_dir_for_result "$result")"
  answer_file="$art_dir/answer.md"
  answer_json="$art_dir/answer.json"
  [[ -d "$art_dir" ]] || fail "missing artifact dir: $art_dir"
  [[ -f "$answer_file" ]] || fail "missing answer.md: $answer_file"
  [[ -f "$answer_json" ]] || fail "missing answer.json: $answer_json"
  grep -F -- "$token" "$answer_file" >/dev/null || fail "answer missing token $token"
  [[ -f "$art_dir/diagnostics/pre-send-visual-gate.png" ]] || fail "missing pre-send screenshot"
  [[ -f "$art_dir/diagnostics/send-after-start-confirmation.dom.json" ]] || fail "missing send confirmation DOM"
  [[ -f "$art_dir/diagnostics/pre-poll-wait-gate.png" ]] || fail "missing pre-poll screenshot"
  [[ -f "$art_dir/diagnostics/poll-terminal-before-artifacts.dom.json" ]] || fail "missing terminal DOM"
  node "$stack_root/scripts/live-smoke-diagnostics-check.mjs" "$art_dir" >"$case_dir/diagnostics-guard.json" \
    || fail "provider limit diagnostics present in $art_dir"
  write_session_summary "$case_dir" "$result" "$token" "$art_dir" "$answer_file" "$answer_json"
}

write_session_summary() {
  local case_dir="$1" result="$2" token="$3" art_dir="$4" answer_file="$5" answer_json="$6"
  {
    printf 'sessionId=%s\n' "$(json_get "$result" sessionId)"
    printf 'conversationUrl=%s\n' "$(json_get "$result" conversationUrl)"
    printf 'slotId=%s\n' "$(json_get "$result" slotId)"
    printf 'accountGroup=%s\n' "$(json_get "$result" accountGroup)"
    printf 'artifactDir=%s\n' "$art_dir"
    printf 'runId=%s\n' "$(json_get "$result" runId)"
    printf 'sendAttempts=%s\n' "$(json_get "$result" sendAttempts 2>/dev/null || printf '')"
    printf 'sendRetryDelaysMs=%s\n' "$(json_get "$result" sendRetryDelaysMs 2>/dev/null || printf '[]')"
    printf 'providerLimitRetryDelaysMs=%s\n' "$(json_get "$result" providerLimitRetryDelaysMs 2>/dev/null || printf '[]')"
    printf 'workerToken=%s\n' "$token"
  } >"$case_dir/session-summary.txt"
  cp "$answer_file" "$case_dir/answer.md"
  cp "$answer_json" "$case_dir/answer.json"
  sha256sum "$answer_file" "$answer_json" >"$case_dir/answer-sha256.txt"
}

source "$live_smoke_lib_dir/validation-downloads.sh"
