#!/usr/bin/env bash
set -euo pipefail

repo_root=${SCAN_SECRETS_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
cd "$repo_root"

credential_pattern='(ghp_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|refresh_token["[:space:]:=]+[A-Za-z0-9._-]{20,}|access_token["[:space:]:=]+[A-Za-z0-9._-]{20,}|TUNNEL_TOKEN=[A-Za-z0-9._-]{40,}|nvapi-[A-Za-z0-9_-]{20,}|nblb_admin_[0-9a-f]{64}|nblb_ds_[0-9a-f]{64})'
private_key_pattern='BEGIN (RSA |OPENSSH |EC |DSA |PRIVATE )?PRIVATE KEY'
forbidden_path_pattern='(^|/)(\.env|auth\.json|github_pat|known_hosts(\.old)?|cert\.pem|private\.pem|id_[^/]+|.*\.sqlite.*|.*\.log)$|(^|/)(state|logs|backups|metadata|mirrors)(/|$)|(^|/)runner/data(/|$)'

tmp_root=$(mktemp -d)
cleanup() {
  rm -rf -- "$tmp_root"
}
trap cleanup EXIT HUP INT TERM

tracked_paths=$tmp_root/tracked-paths
worktree_paths=$tmp_root/worktree-paths
index_entries=$tmp_root/index-entries
blob=$tmp_root/blob

if ! git ls-files -z >"$tracked_paths"; then
  printf '%s\n' 'FAILED: unable to enumerate tracked paths' >&2
  exit 1
fi
if ! git ls-files -co --exclude-standard -z >"$worktree_paths"; then
  printf '%s\n' 'FAILED: unable to enumerate worktree scan candidates' >&2
  exit 1
fi
if ! git ls-files -s -z >"$index_entries"; then
  printf '%s\n' 'FAILED: unable to enumerate Git index blobs' >&2
  exit 1
fi

fail=0

credential_shaped_name() {
  local path=$1
  printf '%s' "$path" | LC_ALL=C grep -qE "$credential_pattern"
}

while IFS= read -r -d '' path; do
  if credential_shaped_name "$path"; then
    printf '%s\n' 'FAILED: credential-shaped filename detected' >&2
    fail=1
  elif printf '%s' "$path" | LC_ALL=C grep -qE "$forbidden_path_pattern"; then
    printf 'FAILED: forbidden tracked path: %q\n' "$path" >&2
    fail=1
  fi
done <"$tracked_paths"

load_worktree_file() {
  local path=$1
  local mode
  local mode_value

  if [ -L "$path" ]; then
    if ! readlink -- "$path" >"$blob" 2>/dev/null; then
      return 2
    fi
  elif [ -f "$path" ]; then
    if ! mode=$(stat -c '%a' -- "$path" 2>/dev/null); then
      return 2
    fi
    mode_value=$((8#$mode))
    if [ $((mode_value & 0444)) -eq 0 ] || [ ! -r "$path" ]; then
      return 2
    fi
    if ! cp -- "$path" "$blob" 2>/dev/null; then
      return 2
    fi
  else
    return 2
  fi
}

scan_loaded_blob() {
  local pattern=$1
  local rc

  if LC_ALL=C grep -qE "$pattern" -- "$blob"; then
    rc=0
  else
    rc=$?
  fi
  case "$rc" in
    0) return 0 ;;
    1) return 1 ;;
    *) return 2 ;;
  esac
}

while IFS= read -r -d '' path; do
  if credential_shaped_name "$path"; then
    printf '%s\n' 'FAILED: credential-shaped filename detected' >&2
    fail=1
  fi
  if [ ! -e "$path" ] && [ ! -L "$path" ]; then
    if git ls-files --error-unmatch -- "$path" >/dev/null 2>&1; then
      continue
    fi
    printf '%s\n' 'FAILED: worktree scan candidate disappeared' >&2
    fail=1
    continue
  fi
  if ! load_worktree_file "$path"; then
    printf '%s\n' 'FAILED: unable to scan worktree file safely' >&2
    fail=1
    continue
  fi
  for finding in private credential; do
    pattern=$private_key_pattern
    diagnostic='private key material in worktree file'
    if [ "$finding" = credential ]; then
      pattern=$credential_pattern
      diagnostic='credential-shaped value in worktree file'
    fi
    if scan_loaded_blob "$pattern"; then
      printf 'FAILED: %s\n' "$diagnostic" >&2
      fail=1
    else
      rc=$?
      if [ "$rc" -ne 1 ]; then
        printf '%s\n' 'FAILED: unable to scan worktree file safely' >&2
        fail=1
      fi
    fi
  done
done <"$worktree_paths"

while IFS= read -r -d '' entry; do
  metadata=${entry%%$'\t'*}
  path=${entry#*$'\t'}
  if [ "$metadata" = "$entry" ]; then
    printf '%s\n' 'FAILED: malformed Git index entry' >&2
    fail=1
    continue
  fi
  read -r mode object_id stage extra <<<"$metadata"
  if [ -n "${extra:-}" ] \
    || [[ ! "$mode" =~ ^[0-7]{6}$ ]] \
    || [[ ! "$object_id" =~ ^[0-9a-f]{40}([0-9a-f]{24})?$ ]] \
    || [[ ! "$stage" =~ ^[0-3]$ ]]; then
    printf '%s\n' 'FAILED: malformed Git index metadata' >&2
    fail=1
    continue
  fi
  if credential_shaped_name "$path"; then
    printf '%s\n' 'FAILED: credential-shaped filename detected' >&2
    fail=1
  fi
  if ! git cat-file blob "$object_id" >"$blob"; then
    printf '%s\n' 'FAILED: unable to read Git index blob' >&2
    fail=1
    continue
  fi
  for finding in private credential; do
    pattern=$private_key_pattern
    diagnostic='private key material in Git index blob'
    if [ "$finding" = credential ]; then
      pattern=$credential_pattern
      diagnostic='credential-shaped value in Git index blob'
    fi
    if LC_ALL=C grep -qE "$pattern" -- "$blob"; then
      rc=0
    else
      rc=$?
    fi
    case "$rc" in
      0)
        printf 'FAILED: %s\n' "$diagnostic" >&2
        fail=1
        ;;
      1) ;;
      *)
        printf '%s\n' 'FAILED: unable to scan Git index blob safely' >&2
        fail=1
        ;;
    esac
  done
done <"$index_entries"

if [ "$fail" -ne 0 ]; then
  exit 1
fi

printf 'Secret scan passed.\n'
