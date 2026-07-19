#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "$0")/.." && pwd)
scanner=$repo_root/scripts/scan-secrets.sh
fixture_root=$(mktemp -d)
cleanup() {
  chmod -R u+rwX -- "$fixture_root" 2>/dev/null || true
  rm -rf -- "$fixture_root"
}
trap cleanup EXIT HUP INT TERM

new_repo() {
  local name=$1
  local repo=$fixture_root/$name
  mkdir -p "$repo"
  git -C "$repo" init -q
  git -C "$repo" config user.email scanner@example.invalid
  git -C "$repo" config user.name scanner-sensor
  printf '%s\n' 'safe baseline' >"$repo/safe.txt"
  git -C "$repo" add safe.txt
  printf '%s\n' "$repo"
}

expect_rejected_worktree_value() {
  local name=$1
  local value=$2
  local repo
  repo=$(new_repo "$name")
  printf '%s\n' "$value" >"$repo/value.txt"
  if SCAN_SECRETS_ROOT="$repo" "$scanner" >/dev/null 2>&1; then
    printf 'Credential scanner accepted worktree fixture: %s\n' "$name" >&2
    exit 1
  fi
}

expect_rejected_worktree_value nvidia "nvapi-$(printf 'a%.0s' {1..40})"
expect_rejected_worktree_value admin "nblb_admin_$(printf 'b%.0s' {1..64})"
expect_rejected_worktree_value downstream "nblb_ds_$(printf 'c%.0s' {1..64})"

staged_repo=$(new_repo staged-index)
printf 'nvapi-%s\n' "$(printf 'd%.0s' {1..40})" >"$staged_repo/safe.txt"
git -C "$staged_repo" add safe.txt
printf '%s\n' 'clean worktree replacement' >"$staged_repo/safe.txt"
if SCAN_SECRETS_ROOT="$staged_repo" "$scanner" >/dev/null 2>&1; then
  printf '%s\n' 'Credential scanner accepted a staged-only credential' >&2
  exit 1
fi

unreadable_repo=$(new_repo unreadable)
printf '%s\n' 'not a credential' >"$unreadable_repo/unreadable.txt"
chmod 000 "$unreadable_repo/unreadable.txt"
if SCAN_SECRETS_ROOT="$unreadable_repo" "$scanner" >/dev/null 2>&1; then
  printf '%s\n' 'Credential scanner accepted an unreadable candidate' >&2
  exit 1
fi
chmod 600 "$unreadable_repo/unreadable.txt"

filename_repo=$(new_repo credential-filename)
credential_filename="nvapi-$(printf 'e%.0s' {1..40}).txt"
printf '%s\n' 'harmless content' >"$filename_repo/$credential_filename"
filename_stderr=$fixture_root/filename.stderr
if SCAN_SECRETS_ROOT="$filename_repo" "$scanner" >/dev/null 2>"$filename_stderr"; then
  printf '%s\n' 'Credential scanner accepted a credential-shaped filename' >&2
  exit 1
fi
if ! grep -Fxq 'FAILED: credential-shaped filename detected' "$filename_stderr"; then
  printf '%s\n' 'Credential filename failure did not use the generic diagnostic' >&2
  exit 1
fi
if grep -Fq "$credential_filename" "$filename_stderr"; then
  printf '%s\n' 'Credential filename failure disclosed the filename' >&2
  exit 1
fi

overlap_repo=$(new_repo credential-filename-overlap)
overlap_filename="nvapi-$(printf 'f%.0s' {1..40}).log"
printf '%s\n' 'harmless content' >"$overlap_repo/$overlap_filename"
git -C "$overlap_repo" add -f "$overlap_filename"
overlap_stderr=$fixture_root/overlap.stderr
if SCAN_SECRETS_ROOT="$overlap_repo" "$scanner" >/dev/null 2>"$overlap_stderr"; then
  printf '%s\n' 'Credential scanner accepted an overlapping forbidden filename' >&2
  exit 1
fi
if ! grep -Fxq 'FAILED: credential-shaped filename detected' "$overlap_stderr"; then
  printf '%s\n' 'Overlapping filename failure did not use the generic diagnostic' >&2
  exit 1
fi
if grep -Fq "$overlap_filename" "$overlap_stderr"; then
  printf '%s\n' 'Overlapping filename failure disclosed the filename' >&2
  exit 1
fi

baseline_repo=$(new_repo baseline)
SCAN_SECRETS_ROOT="$baseline_repo" "$scanner" >/dev/null
printf 'Credential scanner negative sensors passed.\n'
