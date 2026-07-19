#!/usr/bin/env bash
set -Eeuo pipefail
set +x

repo_root=$(cd "$(dirname "$0")/.." && pwd)
helper=$repo_root/scripts/quarantine-hermes-credentials.sh
fixture_parent=$(mktemp -d)
cleanup() {
  chmod -R u+rwX -- "$fixture_parent" 2>/dev/null || true
  rm -rf -- "$fixture_parent"
}
trap cleanup EXIT HUP INT TERM

new_fixture() {
  local name=$1
  local root=$fixture_parent/$name
  mkdir -p "$root/hermes/sessions"
  printf '%s\n' "$root/hermes" >"$root/hermes-mount-sources"
  printf '1 0 8:1 / / rw - btrfs /dev/root rw\n' >"$root/mountinfo"
  printf 'NVIDIA_API_KEY=nvapi-%s\n' "$(printf 'a%.0s' {1..40})" >"$root/hermes/.env"
  chmod 0600 "$root/hermes/.env"
  printf '%s\n' sentinel >"$root/hermes/sentinel.txt"
  printf '%s\n' "$root"
}

run_fixture() {
  NBLB_QUARANTINE_FIXTURE_ROOT=$1 "$helper" "${@:2}"
}

root=$(new_fixture happy)
backup=$root/hermes/.nblb-cutover-backup-fixture
mkdir "$backup"
printf 'NVIDIA_API_KEY=nvapi-%s\n' "$(printf 'b%.0s' {1..40})" >"$backup/env.before"
output=$(run_fixture "$root")
printf '%s\n' "$output" | grep -Fxq 'Hermes legacy credential matches after quarantine: 0'
printf '%s\n' "$output" | grep -Fxq 'Hermes live credential generation: direct-upstream'
generation=$(printf '%s\n' "$output" | sed -n 's/^Legacy quarantine generation: //p')
[[ "$generation" =~ ^manual-quarantine-[0-9]{8}T[0-9]{6}Z-[0-9]+$ ]]
[ ! -e "$backup" ]
[ "$(cat "$root/hermes/sentinel.txt")" = sentinel ]
[ "$(stat -c '%u:%g:%a' "$root/hermes-cutover-backups/$generation")" \
  = "$(id -u):$(id -g):700" ]

retire_dir=$root/hermes-cutover-state/legacy-retirements
mkdir -p "$retire_dir"
retire_receipt=$retire_dir/$generation.receipt
printf '%s\n' \
  'schema_version=1' \
  "generation=$generation" \
  'provider_revocation_confirmed=true' \
  'status=pending' >"$retire_receipt"
chmod 0600 "$retire_receipt"
if run_fixture "$root" retire "$generation" --provider-credential-revoked >/dev/null 2>&1; then
  printf '%s\n' 'Pending retirement deleted an existing generation' >&2
  exit 1
fi
[ -d "$root/hermes-cutover-backups/$generation" ]
rm -f -- "$retire_receipt"

repeat_output=$(run_fixture "$root")
printf '%s\n' "$repeat_output" | grep -Fxq \
  'Hermes legacy credential matches after quarantine: 0'
printf '%s\n' "$repeat_output" | grep -Fxq \
  "Legacy quarantine generation pending retirement: $generation"
if printf '%s\n' "$repeat_output" | grep -q '^Legacy quarantine generation:'; then
  printf '%s\n' 'Idempotent quarantine created an unexpected generation' >&2
  exit 1
fi

retire_output=$(run_fixture "$root" retire "$generation" --provider-credential-revoked)
printf '%s\n' "$retire_output" | grep -Fxq \
  "Retired legacy quarantine generation: $generation"
[ ! -e "$root/hermes-cutover-backups/$generation" ]
[ "$(cat "$root/hermes/sentinel.txt")" = sentinel ]
grep -Fxq 'status=retired' \
  "$root/hermes-cutover-state/legacy-quarantine-generations/$generation.receipt"
retry_retire_output=$(run_fixture "$root" retire "$generation" --provider-credential-revoked)
printf '%s\n' "$retry_retire_output" | grep -Fxq \
  "Retired legacy quarantine generation: $generation"
sed -i 's/^status=complete$/status=pending/' \
  "$root/hermes-cutover-state/legacy-retirements/$generation.receipt"
gap_recovery_output=$(run_fixture "$root" retire "$generation" --provider-credential-revoked)
printf '%s\n' "$gap_recovery_output" | grep -Fxq \
  "Retired legacy quarantine generation: $generation"
grep -Fxq 'status=complete' \
  "$root/hermes-cutover-state/legacy-retirements/$generation.receipt"

signal_root=$(new_fixture post-move-signal)
signal_backup=$signal_root/hermes/.nblb-cutover-backup-signal
mkdir "$signal_backup"
printf 'NVIDIA_API_KEY=nvapi-%s\n' "$(printf 's%.0s' {1..40})" \
  >"$signal_backup/env.before"
fakebin=$signal_root/fakebin
mkdir "$fakebin"
printf '%s\n' \
  '#!/bin/sh' \
  'set -eu' \
  'case " $* " in' \
  '  *.nblb-cutover-backup-*) /bin/mv "$@"; kill -TERM "$PPID"; exit 0 ;;' \
  '  *) exec /bin/mv "$@" ;;' \
  'esac' >"$fakebin/mv"
chmod 0700 "$fakebin/mv"
set +e
signal_output=$(PATH="$fakebin:$PATH" run_fixture "$signal_root" 2>&1)
signal_status=$?
set -e
[ "$signal_status" -eq 143 ]
signal_generation=$(printf '%s\n' "$signal_output" \
  | sed -n 's/^Legacy quarantine generation requires recovery: //p' | tail -1)
[[ "$signal_generation" =~ ^manual-quarantine-[0-9]{8}T[0-9]{6}Z-[0-9]+$ ]]
[ ! -e "$signal_backup" ]
[ -d "$signal_root/hermes-cutover-backups/$signal_generation" ]
grep -Fxq 'status=pending' \
  "$signal_root/hermes-cutover-state/legacy-quarantine-generations/$signal_generation.receipt"
resume_output=$(run_fixture "$signal_root" 2>&1)
printf '%s\n' "$resume_output" | grep -Fxq \
  "Legacy quarantine generation requires recovery: $signal_generation"
printf '%s\n' "$resume_output" | grep -Fxq \
  "Legacy quarantine generation: $signal_generation"
grep -Fxq 'status=complete' \
  "$signal_root/hermes-cutover-state/legacy-quarantine-generations/$signal_generation.receipt"

expect_preflight_rejection() {
  local name=$1
  local kind=$2
  local fixture
  fixture=$(new_fixture "$name")
  local candidate=$fixture/hermes/.nblb-cutover-backup-fixture
  mkdir "$candidate"
  printf '%s\n' safe >"$candidate/env.before"
  case "$kind" in
    symlink) ln -s env.before "$candidate/link" ;;
    hardlink) ln "$candidate/env.before" "$candidate/second-link" ;;
    fifo) mkfifo "$candidate/unsafe.fifo" ;;
    nested-mount)
      printf '1 0 8:1 / / rw - btrfs /dev/root rw\n' >"$fixture/mountinfo"
      printf '2 1 8:1 / %s rw - btrfs /dev/root rw\n' "$candidate" >>"$fixture/mountinfo"
      ;;
    *) printf '%s\n' 'Unknown fixture kind' >&2; exit 64 ;;
  esac
  if run_fixture "$fixture" >/dev/null 2>&1; then
    printf 'Unsafe quarantine fixture passed: %s\n' "$name" >&2
    exit 1
  fi
  [ -d "$candidate" ]
  [ "$(cat "$fixture/hermes/sentinel.txt")" = sentinel ]
}

expect_preflight_rejection symlink symlink
expect_preflight_rejection hardlink hardlink
expect_preflight_rejection fifo fifo
expect_preflight_rejection nested-mount nested-mount

sessions_alias_root=$(new_fixture sessions-symlink)
outside_sessions=$(mktemp -d)
rm -rf -- "$sessions_alias_root/hermes/sessions"
ln -s "$outside_sessions" "$sessions_alias_root/hermes/sessions"
if run_fixture "$sessions_alias_root" >/dev/null 2>&1; then
  printf '%s\n' 'Symlinked Hermes sessions directory was accepted' >&2
  exit 1
fi
rm -rf -- "$outside_sessions"

overlap_root=$(new_fixture mount-overlap)
mkdir -p "$overlap_root/hermes-cutover-backups/mounted-child"
printf '%s\n' "$overlap_root/hermes-cutover-backups/mounted-child" \
  >"$overlap_root/hermes-mount-sources"
if run_fixture "$overlap_root" >/dev/null 2>&1; then
  printf '%s\n' 'Nested quarantine mount source was accepted' >&2
  exit 1
fi
[ "$(cat "$overlap_root/hermes/sentinel.txt")" = sentinel ]

alias_root=$(new_fixture physical-alias)
printf '2 1 8:1 %s %s rw - btrfs /dev/root rw\n' \
  "$alias_root/hermes/host-only-alias" \
  "$alias_root/hermes-cutover-backups" >>"$alias_root/mountinfo"
if run_fixture "$alias_root" >/dev/null 2>&1; then
  printf '%s\n' 'Physical Hermes bind alias was accepted' >&2
  exit 1
fi
[ "$(cat "$alias_root/hermes/sentinel.txt")" = sentinel ]

multi_root=$(new_fixture multiple-live-credentials)
printf '# stale nvapi-%s\n' "$(printf 'z%.0s' {1..40})" >>"$multi_root/hermes/.env"
if run_fixture "$multi_root" >/dev/null 2>&1; then
  printf '%s\n' 'Multiple live upstream credentials were accepted' >&2
  exit 1
fi
[ "$(cat "$multi_root/hermes/sentinel.txt")" = sentinel ]

printf '%s\n' 'Hermes quarantine behavioral sensors passed.'
