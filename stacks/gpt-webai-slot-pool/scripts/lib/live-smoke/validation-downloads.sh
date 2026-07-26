validate_downloads() {
  local case_dir="$1" result="$2" min_files="$3" art_dir downloads count
  art_dir="$(artifact_dir_for_result "$result")"
  downloads="$art_dir/downloads"
  [[ -d "$downloads" ]] || fail "missing downloads dir: $downloads"
  find "$downloads" -maxdepth 1 -type f -printf '%f\n' | sort >"$case_dir/downloads.txt"
  count="$(wc -l <"$case_dir/downloads.txt")"
  [[ "$count" -ge "$min_files" ]] || fail "expected at least $min_files downloads, got $count"
  find "$downloads" -maxdepth 1 -type f -exec sha256sum {} + | sort >"$case_dir/download-sha256.txt"
  python3 - "$downloads" >"$case_dir/download-integrity.txt" <<'PY'
import pathlib, sys, zipfile
root = pathlib.Path(sys.argv[1])
for path in sorted(root.iterdir()):
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as zf:
            bad = zf.testzip()
            if bad:
                raise SystemExit(f"bad zip member {bad} in {path}")
        print(f"zip_ok {path.name}")
    elif path.stat().st_size <= 0:
        raise SystemExit(f"empty download {path}")
    else:
        print(f"file_ok {path.name}")
PY
}

run_lifecycle() {
  local case_dir="$1" token="$2" prompt="$3"
  shift 3
  local rc files=("$@") cmd=("$lifecycle" run --kind pro --poll-timeout-seconds "$smoke_poll_timeout_seconds")
  mkdir -p "$case_dir"
  write_env_file "$case_dir/env.txt"
  capture_preflight "$case_dir" "$(basename "$case_dir")-preflight-$stamp"
  for file in "${files[@]}"; do
    cmd+=(--file "$file")
  done
  cmd+=(--prompt "$prompt")
  write_command_file "$case_dir/command.txt" "${cmd[@]}"
  set +e
  "${cmd[@]}" >"$case_dir/run.out" 2>"$case_dir/run.err"
  rc="$?"
  set -e
  printf '%s\n' "$rc" >"$case_dir/run.rc"
  if [[ "$rc" -ne 0 ]]; then
    record_status "$case_dir" final
    capture_container_status "$case_dir/containers.final.out" "$case_dir/containers.final.err"
    fail "lifecycle command failed rc=$rc: $case_dir"
  fi
  validate_run_result "$case_dir" "$case_dir/run.out" "$token"
  record_status "$case_dir" final
  if [[ "${GPT_WEBAI_SMOKE_ASSERT_CLEAN_STATUS:-1}" == 1 ]]; then
    assert_clean_status "$case_dir/status.final.json"
  fi
  capture_container_status "$case_dir/containers.final.out" "$case_dir/containers.final.err"
}
