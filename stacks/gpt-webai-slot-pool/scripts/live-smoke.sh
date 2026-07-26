#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
stack_root="$repo_root/stacks/gpt-webai-slot-pool"
lifecycle="${GPT_WEBAI_LIFECYCLE:-$stack_root/bin/gpt-webai-lifecycle}"
state_dir="${GPT_WEBAI_STATE_ROOT:-${XDG_STATE_HOME:-$HOME/.local/state}/gpt-webai-lifecycle/r13}"
slot_count="${GPT_WEBAI_SLOT_COUNT:-10}"
parallel_count=5
cases=()
original_args=("$@")
smoke_poll_timeout_seconds="${GPT_WEBAI_SMOKE_POLL_TIMEOUT_SECONDS:-300}"
live_smoke_lib_dir="$stack_root/scripts/lib/live-smoke"

export GPT_WEBAI_STATE_ROOT="$state_dir"

source "$live_smoke_lib_dir/common.sh"
source "$live_smoke_lib_dir/validation.sh"
source "$live_smoke_lib_dir/cases.sh"
source "$live_smoke_lib_dir/parallel.sh"

parse_args() {
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
}

[[ "$smoke_poll_timeout_seconds" =~ ^[0-9]+$ && "$smoke_poll_timeout_seconds" -gt 0 ]] || fail 'GPT_WEBAI_SMOKE_POLL_TIMEOUT_SECONDS must be a positive integer'
parse_args "$@"
[[ "${#cases[@]}" -gt 0 ]] || cases=(qa-fast)

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
evidence_root="${GPT_WEBAI_SMOKE_EVIDENCE:-$repo_root/.omo/evidence/gpt-webai-slot-pool-live-smoke/$stamp}"
mkdir -p "$evidence_root/bin"
ln -sfn "$lifecycle" "$evidence_root/bin/gpt-webai-lifecycle"
export PATH="$evidence_root/bin:/home/dongwonttuna/.local/bin:$PATH"

trap cleanup_on_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

write_run_metadata
run_requested_cases
record_status "$evidence_root" final
assert_clean_status "$evidence_root/status.final.json"
capture_container_status "$evidence_root/containers.final.out" "$evidence_root/containers.final.err"
printf 'evidence=%s\n' "$evidence_root"
