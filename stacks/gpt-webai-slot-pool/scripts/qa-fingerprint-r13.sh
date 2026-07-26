#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 1 || "$1" != "--print" ]]; then
  printf 'usage: %s --print\n' "$0" >&2
  exit 2
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
stack_root="$(cd -- "$script_dir/.." && pwd -P)"
repo_root="$(git -C "$stack_root" rev-parse --show-toplevel)"
repo_prefix="${stack_root#"$repo_root"/}"
work_file="$(mktemp "${TMPDIR:-/tmp}/pr72-fingerprint-r13.XXXXXX")"
sorted_file="$(mktemp "${TMPDIR:-/tmp}/pr72-fingerprint-r13-sorted.XXXXXX")"
trap 'rm -f -- "$work_file" "$sorted_file"' EXIT

declare -A seen=()

add_entry() {
  local display_path="$1"
  local disk_path
  if [[ "$display_path" = /* ]]; then
    disk_path="$display_path"
  else
    disk_path="$repo_root/$display_path"
  fi
  [[ "$display_path" != *$'\n'* && "$display_path" != *$'\r'* ]] || {
    printf 'unsafe fingerprint path: %q\n' "$display_path" >&2
    exit 1
  }
  [[ -f "$disk_path" ]] || {
    printf 'required fingerprint input is missing or non-regular: %s\n' "$display_path" >&2
    exit 1
  }
  if [[ -n "${seen[$display_path]+present}" ]]; then
    return 0
  fi
  seen["$display_path"]=1
  local digest
  digest="$(sha256sum -- "$disk_path")"
  digest="${digest%% *}"
  printf '%s  %s\n' "$digest" "$display_path" >> "$work_file"
}

add_tracked_entry() {
  local display_path="$1"
  local disk_path="$repo_root/$display_path"
  [[ "$display_path" != *$'\n'* && "$display_path" != *$'\r'* ]] || {
    printf 'unsafe fingerprint path: %q\n' "$display_path" >&2
    exit 1
  }
  if [[ -n "${seen[$display_path]+present}" ]]; then
    return 0
  fi
  seen["$display_path"]=1
  if [[ ! -e "$disk_path" && ! -L "$disk_path" ]]; then
    printf 'deleted  %s\n' "$display_path" >> "$work_file"
    return 0
  fi
  [[ -f "$disk_path" ]] || {
    printf 'tracked fingerprint input is non-regular: %s\n' "$display_path" >&2
    exit 1
  }
  local digest
  digest="$(sha256sum -- "$disk_path")"
  digest="${digest%% *}"
  printf '%s  %s\n' "$digest" "$display_path" >> "$work_file"
}

while IFS= read -r -d '' path; do
  [[ "$path" == .omo/evidence/* ]] || add_tracked_entry "$path"
done < <(git -C "$repo_root" ls-files -z)

while IFS= read -r -d '' path; do
  case "$path" in
    */.omo/evidence/*|*/node_modules/*|*/target/*|*/__pycache__/*|*/.fable-sol/*) ;;
    *) add_entry "$path" ;;
  esac
done < <(git -C "$repo_root" ls-files --others --exclude-standard -z -- "$repo_prefix")

while IFS= read -r -d '' path; do
  add_entry "${path#"$repo_root"/}"
done < <(find "$repo_root/.omo/plans/pr72-canonical-design" -maxdepth 1 -type f -print0)

add_entry "$HOME/.local/bin/gptpro"
add_entry "$HOME/.local/bin/gptxhigh"
add_entry "$repo_prefix/bin/gpt-webai-lifecycle"
add_entry "$repo_prefix/bin/gpt-webai-lifecycle-rust"

: "${PR72_GOAL_PATH:?PR72_GOAL_PATH is required}"
: "${PR72_HANDOFF_PATH:?PR72_HANDOFF_PATH is required}"
add_entry "$PR72_GOAL_PATH"
add_entry "$PR72_HANDOFF_PATH"
add_entry "$HOME/.codex/AGENTS.md"
add_entry "$HOME/.codex/prompts/gpt-delegation-prelude.md"
add_entry "$HOME/.codex/runbooks/gpt-webai-lifecycle.md"
add_entry "AGENTS.md"
add_entry "$repo_prefix/README.md"
add_entry "$repo_prefix/SMOKE_TESTS.md"
add_entry "$repo_prefix/docs/gpt-webai-lifecycle-runbook.md"

LC_ALL=C sort -- "$work_file" > "$sorted_file"
sha256sum -- "$sorted_file" | awk '{print $1}'
