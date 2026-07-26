parallel_worker_text() {
  local index="$1" case_dir="$2" token="LIVE_PARALLEL_TEXT_${stamp}_$1"
  run_lifecycle "$case_dir/worker-$index" "$token" "Reply with exactly one line and no extra text: $token"
}

parallel_worker_files() {
  local index="$1" case_dir="$2" file_count="$3" token="LIVE_PARALLEL_FILES_${stamp}_$1" files=()
  mkdir -p "$case_dir/worker-$index/inputs"
  for n in $(seq 1 "$file_count"); do
    local file="$case_dir/worker-$index/inputs/canary-$n.md"
    printf 'CANARY_OK_%s: %s_%s\n' "$n" "$token" "$n" >"$file"
    files+=("$file")
  done
  sha256sum "${files[@]}" >"$case_dir/worker-$index/attachment-sha256.txt"
  run_lifecycle "$case_dir/worker-$index" "$token" "Read all attached files and reply with every CANARY_OK line." "${files[@]}"
}

case_live_parallel() {
  local mode="$1" width="$2" file_count="${3:-0}"
  local case_dir="$evidence_root/live-parallel-$mode-w$width" i failure=0
  local -a worker_pids=()
  note "live-parallel-$mode width=$width"
  mkdir -p "$case_dir"
  capture_preflight "$case_dir" "live-parallel-$mode-w$width-preflight-$stamp"
  for i in $(seq 1 "$width"); do
    mkdir -p "$case_dir/worker-$i"
    case "$mode" in
      text)
        (GPT_WEBAI_SMOKE_ASSERT_CLEAN_STATUS=0 parallel_worker_text "$i" "$case_dir") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
        worker_pids+=("$!")
        ;;
      attachment|attachments)
        (GPT_WEBAI_SMOKE_ASSERT_CLEAN_STATUS=0 parallel_worker_files "$i" "$case_dir" "$file_count") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
        worker_pids+=("$!")
        ;;
      mixed)
        if (( i % 2 == 0 )); then
          (GPT_WEBAI_SMOKE_ASSERT_CLEAN_STATUS=0 parallel_worker_files "$i" "$case_dir" "$file_count") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
          worker_pids+=("$!")
        else
          (GPT_WEBAI_SMOKE_ASSERT_CLEAN_STATUS=0 parallel_worker_text "$i" "$case_dir") >"$case_dir/worker-$i/stdout.log" 2>"$case_dir/worker-$i/stderr.log" &
          worker_pids+=("$!")
        fi
        ;;
      *) fail "unknown parallel mode: $mode" ;;
    esac
  done
  for pid in "${worker_pids[@]}"; do
    wait "$pid" || failure=1
  done
  [[ "$failure" -eq 0 ]] || fail "one or more parallel workers failed in $case_dir"
  find "$case_dir" -path '*/session-summary.txt' -print -exec cat {} \; >"$case_dir/session-summaries.txt"
  node "$stack_root/scripts/live-smoke-slot-summary.mjs" "$case_dir" "$width"
  record_status "$case_dir" final
  assert_clean_status "$case_dir/status.final.json"
}

case_qa_fast() {
  case_sentinel_start_unconfirmed
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
  case_sentinel_start_unconfirmed
  case_live_text
  case_live_attachment
  case_live_attachments
  case_live_resume
  case_live_artifact_download
  case_live_artifact_previous
  case_live_artifact_multiple
  for width in 1 5 10; do
    case_live_parallel text "$width"
    case_live_parallel attachment "$width" 1
    case_live_parallel attachments "$width" 3
  done
  case_live_parallel mixed 10 3
}

run_requested_cases() {
  local expanded=() case_name
  for case_name in "${cases[@]}"; do
    [[ "$case_name" == all ]] && expanded+=(qa-full) || expanded+=("$case_name")
  done
  for case_name in "${expanded[@]}"; do
    case "$case_name" in
      qa-fast) case_qa_fast ;;
      qa-full) case_qa_full ;;
      sentinel-start-unconfirmed) case_sentinel_start_unconfirmed ;;
      live-text) case_live_text ;;
      live-attachment) case_live_attachment ;;
      live-attachments) case_live_attachments ;;
      live-artifact-download) case_live_artifact_download ;;
      live-artifact-previous) case_live_artifact_previous ;;
      live-artifact-multiple) case_live_artifact_multiple ;;
      live-resume) case_live_resume ;;
      live-parallel-text) case_live_parallel text "$parallel_count" ;;
      live-parallel-attachment) case_live_parallel attachment "$parallel_count" 1 ;;
      live-parallel-attachments) case_live_parallel attachments "$parallel_count" 3 ;;
      live-parallel-mixed) case_live_parallel mixed "$parallel_count" 3 ;;
      *) fail "unknown case: $case_name" ;;
    esac
  done
}
