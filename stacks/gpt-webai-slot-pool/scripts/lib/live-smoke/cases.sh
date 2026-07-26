run_cargo_exact_test_smoke() {
  local case_dir="$1" test_target="$2" test_name="$3" expected_tests="${4:-1}" rc
  local -a cmd=(cargo test --test "$test_target" "$test_name" -- --exact)
  mkdir -p "$case_dir"
  write_command_file "$case_dir/command.txt" "${cmd[@]}"
  set +e
  (cd "$repo_root/stacks/gpt-webai-slot-pool" && "${cmd[@]}") >"$case_dir/out" 2>"$case_dir/err"
  rc="$?"
  set -e
  printf '%s\n' "$rc" >"$case_dir/rc"
  [[ "$rc" -eq 0 ]] || fail "cargo exact smoke failed rc=$rc: $case_dir"
  if ! python3 - "$case_dir/out" "$expected_tests" "$test_name" >"$case_dir/test-count.txt" <<'PY'
import re
import sys
from pathlib import Path

out_file = Path(sys.argv[1])
expected = int(sys.argv[2])
test_name = sys.argv[3]
text = out_file.read_text(encoding="utf-8", errors="replace")
counts = [int(match.group(1)) for match in re.finditer(r"^running ([0-9]+) tests?$", text, re.MULTILINE)]
observed = sum(counts)
print(f"expected={expected}")
print(f"observed={observed}")
print("running_lines=" + (",".join(str(count) for count in counts) if counts else "none"))
marker = f"test {test_name} ... ok"
if observed != expected:
    raise SystemExit(f"expected exactly {expected} executed test(s), observed {observed}")
if expected == 1 and marker not in text:
    raise SystemExit(f"missing expected passing test marker: {marker}")
PY
  then
    fail "cargo exact smoke did not execute the expected test count: $case_dir"
  fi
}

case_sentinel_start_unconfirmed() {
  note sentinel-start-unconfirmed
  local case_dir="$evidence_root/sentinel-start-unconfirmed"
  run_cargo_exact_test_smoke \
    "$case_dir" \
    request \
    request::retry::send::unconfirmed::sent_root_url_is_start_unconfirmed_and_does_not_retry \
    1
}

case_live_text() {
  note live-text
  local token="LIVE_TEXT_${stamp}"
  run_lifecycle "$evidence_root/live-text" "$token" "Reply with exactly one line and no extra text: $token"
}

case_live_attachment() {
  note live-attachment
  local case_dir="$evidence_root/live-attachment" token="LIVE_ATTACHMENT_${stamp}" file
  mkdir -p "$case_dir/inputs"
  file="$case_dir/inputs/ATTACHMENT_CANARY.md"
  printf 'CANARY_OK: %s\n' "$token" >"$file"
  sha256sum "$file" >"$case_dir/attachment-sha256.txt"
  run_lifecycle "$case_dir" "$token" "Read the attached file and reply with exactly the CANARY_OK line." "$file"
}

case_live_attachments() {
  note live-attachments
  local case_dir="$evidence_root/live-attachments" token="LIVE_ATTACHMENTS_${stamp}" files=()
  mkdir -p "$case_dir/inputs"
  for i in 1 2 3; do
    local file="$case_dir/inputs/ATTACHMENT_CANARY_$i.md"
    printf 'CANARY_OK_%s: %s_%s\n' "$i" "$token" "$i" >"$file"
    files+=("$file")
  done
  sha256sum "${files[@]}" >"$case_dir/attachment-sha256.txt"
  run_lifecycle "$case_dir" "$token" "Read all attached files and reply with every CANARY_OK line, one per line." "${files[@]}"
  for i in 1 2 3; do
    grep -F "CANARY_OK_$i: ${token}_$i" "$case_dir/answer.md" >/dev/null || fail "missing multi-file canary $i"
  done
}

artifact_prompt() {
  local token="$1" kind="${2:-zip}"
  if [[ "$kind" == multiple ]]; then
    printf 'Read the attached seed file for the exact canary line. Use Python/data analysis to write three files under /mnt/data and return them as downloadable attachments, not plain filename text. Name the files pr72-%s-1.txt, pr72-%s-2.txt, and pr72-%s-3.txt. Each file must contain ARTIFACT_CANARY: %s. Do not reply with ARTIFACT_DONE until the actual downloadable file controls are attached. If you cannot attach downloadable files, reply exactly ARTIFACT_UNAVAILABLE: %s. After the files are attached, reply with exactly ARTIFACT_DONE: %s.\n' "$token" "$token" "$token" "$token" "$token" "$token"
  else
    printf 'Read the attached seed file for the exact canary line. Use Python/data analysis to write files under /mnt/data and return them as downloadable attachments, not plain filename text. Create a zip file named pr72-%s.zip containing canary.txt with exact line ARTIFACT_CANARY: %s. Also create a downloadable sha256 sidecar named pr72-%s.zip.sha256. Do not reply with ARTIFACT_DONE until both actual downloadable file controls are attached. If you cannot attach downloadable files, reply exactly ARTIFACT_UNAVAILABLE: %s. After both files are attached, reply with exactly ARTIFACT_DONE: %s.\n' "$token" "$token" "$token" "$token" "$token"
  fi
}

write_artifact_seed_file() {
  local case_dir="$1" token="$2" file
  mkdir -p "$case_dir/inputs"
  file="$case_dir/inputs/artifact-seed.md"
  printf 'ARTIFACT_CANARY: %s\n' "$token" >"$file"
  sha256sum "$file" >"$case_dir/attachment-sha256.txt"
  printf '%s\n' "$file"
}

case_live_artifact_download() {
  note live-artifact-download
  local case_dir="$evidence_root/live-artifact-download" token="LIVE_ARTIFACT_${stamp}" file
  file="$(write_artifact_seed_file "$case_dir" "$token")"
  run_lifecycle "$case_dir" "ARTIFACT_DONE: $token" "$(artifact_prompt "$token")" "$file"
  validate_run_result "$case_dir" "$case_dir/run.out" "ARTIFACT_DONE: $token" 1
  validate_downloads "$case_dir" "$case_dir/run.out" 1
}

case_live_artifact_previous() {
  note live-artifact-previous
  local case_dir="$evidence_root/live-artifact-previous" token="LIVE_ARTIFACT_PREVIOUS_${stamp}" session_id file
  file="$(write_artifact_seed_file "$case_dir" "$token")"
  run_lifecycle "$case_dir" "ARTIFACT_DONE: $token" "$(artifact_prompt "$token")" "$file"
  session_id="$(json_get "$case_dir/run.out" sessionId)"
  write_command_file "$case_dir/download.command" "$lifecycle" download --kind pro --session "$session_id"
  "$lifecycle" download --kind pro --session "$session_id" >"$case_dir/download.out" 2>"$case_dir/download.err"
  printf '%s\n' "$?" >"$case_dir/download.rc"
  validate_run_result "$case_dir" "$case_dir/download.out" "ARTIFACT_DONE: $token" 1
  validate_downloads "$case_dir" "$case_dir/download.out" 1
}

case_live_artifact_multiple() {
  note live-artifact-multiple
  local case_dir="$evidence_root/live-artifact-multiple" token="LIVE_ARTIFACT_MULTIPLE_${stamp}" file
  file="$(write_artifact_seed_file "$case_dir" "$token")"
  run_lifecycle "$case_dir" "ARTIFACT_DONE: $token" "$(artifact_prompt "$token" multiple)" "$file"
  validate_run_result "$case_dir" "$case_dir/run.out" "ARTIFACT_DONE: $token" 3
  validate_downloads "$case_dir" "$case_dir/run.out" 3
}

case_live_resume() {
  note live-resume
  local case_dir="$evidence_root/live-resume" token="LIVE_RESUME_${stamp}" session_id
  run_lifecycle "$case_dir" "$token" "Reply with exactly one line and no extra text: $token"
  session_id="$(json_get "$case_dir/run.out" sessionId)"
  write_command_file "$case_dir/show.command" "$lifecycle" show --kind pro --session "$session_id"
  "$lifecycle" show --kind pro --session "$session_id" >"$case_dir/show.out" 2>"$case_dir/show.err"
  printf '%s\n' "$?" >"$case_dir/show.rc"
  [[ "$(json_get "$case_dir/show.out" sessionId)" == "$session_id" ]] || fail 'show returned wrong session'
  write_command_file "$case_dir/resume.command" "$lifecycle" resume --kind pro --session "$session_id"
  "$lifecycle" resume --kind pro --session "$session_id" >"$case_dir/resume.out" 2>"$case_dir/resume.err"
  printf '%s\n' "$?" >"$case_dir/resume.rc"
  [[ "$(json_get "$case_dir/resume.out" sessionId)" == "$session_id" ]] || fail 'resume returned wrong session'
  [[ "$(json_get "$case_dir/resume.out" status)" == done ]] || fail 'resume did not return terminal done status'
  [[ "$(json_get "$case_dir/resume.out" providerStatus)" == done ]] || fail 'resume providerStatus is not done'
  json_number_ge "$case_dir/resume.out" answerTextLen 1 || fail 'resume did not recover a non-empty answer'
  record_status "$case_dir" after-resume
  assert_clean_status "$case_dir/status.after-resume.json"
}
