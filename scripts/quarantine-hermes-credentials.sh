#!/usr/bin/env bash
set -Eeuo pipefail
set +x

fixture_root=${NBLB_QUARANTINE_FIXTURE_ROOT:-}
owner_uid=0
owner_gid=0
if [ -n "$fixture_root" ]; then
  case "$fixture_root" in /*) ;; *) printf '%s\n' 'Fixture root must be absolute' >&2; exit 64 ;; esac
  [ -d "$fixture_root" ] && [ ! -L "$fixture_root" ] \
    || { printf '%s\n' 'Fixture root is unsafe' >&2; exit 64; }
  fixture_root=$(readlink -f -- "$fixture_root")
  owner_uid=$(id -u)
  owner_gid=$(id -g)
elif [ "$(id -u)" -ne 0 ]; then
  printf '%s\n' 'quarantine-hermes-credentials must run as root' >&2
  exit 77
fi

command=quarantine
generation=
if [ "$#" -ne 0 ]; then
  if [ "$#" -eq 3 ] \
    && [ "$1" = retire ] \
    && [ "$3" = --provider-credential-revoked ]; then
    command=retire
    generation=$2
  else
    printf '%s\n' \
      'usage: quarantine-hermes-credentials.sh [retire GENERATION --provider-credential-revoked]' >&2
    exit 64
  fi
fi

if [ -n "$fixture_root" ]; then
  source_root=$fixture_root/hermes
  quarantine_root=$fixture_root/hermes-cutover-backups
  state_root=$fixture_root/hermes-cutover-state
  mount_inventory=$fixture_root/hermes-mount-sources
  mountinfo_path=$fixture_root/mountinfo
else
  source_root=/opt/agent-apps/data/hermes
  quarantine_root=/opt/nvidia-build-lb/hermes-cutover-backups
  state_root=/opt/nvidia-build-lb/hermes-cutover-state
  mount_inventory=
  mountinfo_path=/proc/self/mountinfo
fi
credential_pattern='nvapi-[A-Za-z0-9_-]{20,}'

for path in "$source_root" "$(dirname "$quarantine_root")"; do
  [ -d "$path" ] && [ ! -L "$path" ] || {
    printf '%s\n' 'Required host directory is missing or unsafe' >&2
    exit 1
  }
done
for path in "$quarantine_root" "$state_root"; do
  if [ -L "$path" ] || { [ -e "$path" ] && [ ! -d "$path" ]; }; then
    printf '%s\n' 'Host-only cutover directory is unsafe' >&2
    exit 1
  fi
  if [ ! -e "$path" ]; then
    install -d -o "$owner_uid" -g "$owner_gid" -m 0700 "$path"
  fi
  [ -d "$path" ] && [ ! -L "$path" ] \
    && [ "$(stat -c '%u:%g:%a' -- "$path")" = "$owner_uid:$owner_gid:700" ] || {
      printf '%s\n' 'Host-only cutover directory metadata is invalid' >&2
      exit 1
    }
done

lock_path=$state_root/cutover.lock
if [ -L "$lock_path" ]; then
  printf '%s\n' 'Cutover lock path is unsafe' >&2
  exit 1
fi
if [ ! -e "$lock_path" ]; then
  install -o "$owner_uid" -g "$owner_gid" -m 0600 /dev/null "$lock_path"
fi
[ -f "$lock_path" ] && [ ! -L "$lock_path" ] \
  && [ "$(stat -c '%u:%g:%a:%h' -- "$lock_path")" = "$owner_uid:$owner_gid:600:1" ] || {
    printf '%s\n' 'Cutover lock metadata is invalid' >&2
    exit 1
  }
exec 9<>"$lock_path"
flock -x 9

mounts=$(mktemp)
matches=$(mktemp)
errors=$(mktemp)
entries=$(mktemp)
generation_receipt_root=$state_root/legacy-quarantine-generations
generation_receipt=
generation_status=
destination=
finish() {
  status=$?
  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] && [ -n "$generation" ]; then
    if [ -n "$destination" ] && [ -d "$destination" ] && [ ! -L "$destination" ]; then
      chown -R --no-dereference "$owner_uid:$owner_gid" "$destination" 2>/dev/null || true
      find -P "$destination" -type d -exec chmod 0700 {} + 2>/dev/null || true
      find -P "$destination" -type f -exec chmod 0600 {} + 2>/dev/null || true
      sync -f "$destination" 2>/dev/null || true
    fi
    sync -f "$quarantine_root" 2>/dev/null || true
    printf 'Legacy quarantine generation requires recovery: %s\n' \
      "$generation" >&2
  fi
  rm -f -- "$mounts" "$matches" "$errors" "$entries"
  exit "$status"
}
trap finish EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

quarantine_real=$(readlink -f -- "$quarantine_root")
state_real=$(readlink -f -- "$state_root")
if [ -n "$fixture_root" ]; then
  [ -f "$mount_inventory" ] && [ ! -L "$mount_inventory" ] \
    || { printf '%s\n' 'Fixture mount inventory is unsafe' >&2; exit 64; }
  cp -- "$mount_inventory" "$mounts"
else
  docker inspect --format '{{range .Mounts}}{{println .Source}}{{end}}' agent-hermes >"$mounts"
fi
while IFS= read -r mount_source; do
  [ -n "$mount_source" ] || continue
  mount_real=$(readlink -f -- "$mount_source")
  case "$quarantine_real/" in "$mount_real/"*)
    printf '%s\n' 'Quarantine root is mounted into Hermes' >&2
    exit 1
  esac
  case "$mount_real/" in "$quarantine_real/"*)
    printf '%s\n' 'A quarantine child is mounted into Hermes' >&2
    exit 1
  esac
  case "$state_real/" in "$mount_real/"*)
    printf '%s\n' 'Cutover state root is mounted into Hermes' >&2
    exit 1
  esac
  case "$mount_real/" in "$state_real/"*)
    printf '%s\n' 'A cutover state child is mounted into Hermes' >&2
    exit 1
  esac
done <"$mounts"

python3 - "$mountinfo_path" "$quarantine_real" "$state_real" "$mounts" <<'PY'
import os
import re
import sys
from pathlib import Path

mountinfo_path, quarantine_root, state_root, sources_path = sys.argv[1:]


def unescape(value: str) -> str:
    return re.sub(r'\\([0-7]{3})', lambda match: chr(int(match.group(1), 8)), value)


mounts: list[tuple[str, Path, Path]] = []
for line in Path(mountinfo_path).read_text(encoding='utf-8').splitlines():
    fields = line.split()
    if len(fields) < 10 or '-' not in fields:
        raise SystemExit('Mount inventory is invalid')
    mounts.append((fields[2], Path(unescape(fields[3])), Path(unescape(fields[4]))))


def identity(raw_path: str) -> tuple[str, Path]:
    path = Path(os.path.abspath(raw_path))
    candidates = [entry for entry in mounts if path == entry[2] or path.is_relative_to(entry[2])]
    if not candidates:
        raise SystemExit('Mount identity is unavailable')
    device, root, mountpoint = max(candidates, key=lambda entry: len(entry[2].parts))
    relative = path.relative_to(mountpoint)
    physical = Path(os.path.normpath(root / relative))
    return device, physical


def overlaps(first: tuple[str, Path], second: tuple[str, Path]) -> bool:
    if first[0] != second[0]:
        return False
    return (
        first[1] == second[1]
        or first[1].is_relative_to(second[1])
        or second[1].is_relative_to(first[1])
    )


host_roots = [identity(quarantine_root), identity(state_root)]
source_roots = [
    identity(line)
    for line in Path(sources_path).read_text(encoding='utf-8').splitlines()
    if line
]
if overlaps(host_roots[0], host_roots[1]) or any(
    overlaps(host_root, source_root)
    for host_root in host_roots
    for source_root in source_roots
):
    raise SystemExit('Host-only cutover root aliases a Hermes mount')
PY

validate_tree() {
  local path=$1
  local linked
  local special
  local hardlinked
  if ! linked=$(find -P "$path" -type l -print -quit) \
    || ! special=$(find -P "$path" ! -type d ! -type f -print -quit) \
    || ! hardlinked=$(find -P "$path" -type f -links +1 -print -quit); then
    printf '%s\n' 'Unable to validate a legacy credential artifact' >&2
    exit 1
  fi
  if [ -L "$path" ] || [ -n "$linked" ] || [ -n "$special" ] || [ -n "$hardlinked" ]; then
    printf '%s\n' 'Legacy credential artifact has an unsafe entry' >&2
    exit 1
  fi
}

reject_nested_mounts() {
  python3 - "$1" "$mountinfo_path" <<'PY'
import os
import re
import sys
from pathlib import Path

target = Path(os.path.abspath(sys.argv[1]))
for line in Path(sys.argv[2]).read_text(encoding='utf-8').splitlines():
    fields = line.split()
    if len(fields) < 10 or '-' not in fields:
        raise SystemExit('Mount inventory is invalid')
    value = re.sub(r'\\([0-7]{3})', lambda match: chr(int(match.group(1), 8)), fields[4])
    mountpoint = Path(os.path.abspath(value))
    if mountpoint == target or mountpoint.is_relative_to(target):
        raise SystemExit('Legacy credential artifact contains a nested mount')
PY
}

validate_private_tree() {
  local path=$1
  local entry
  local expected
  if ! find -P "$path" -mindepth 0 -print0 >"$entries"; then
    printf '%s\n' 'Unable to enumerate a quarantine generation' >&2
    exit 1
  fi
  while IFS= read -r -d '' entry; do
    expected=$owner_uid:$owner_gid:600:1
    if [ -d "$entry" ]; then
      expected=$owner_uid:$owner_gid:700:2
      if [ "$(stat -c '%u:%g:%a' -- "$entry")" = "$owner_uid:$owner_gid:700" ]; then
        continue
      fi
    elif [ "$(stat -c '%u:%g:%a:%h' -- "$entry")" = "$expected" ]; then
      continue
    fi
    printf '%s\n' 'Quarantine generation metadata is invalid' >&2
    exit 1
  done <"$entries"
}

ensure_generation_receipt_root() {
  if [ -L "$generation_receipt_root" ] \
    || { [ -e "$generation_receipt_root" ] && [ ! -d "$generation_receipt_root" ]; }; then
    printf '%s\n' 'Legacy quarantine receipt directory is unsafe' >&2
    exit 1
  fi
  if [ ! -e "$generation_receipt_root" ]; then
    install -d -o "$owner_uid" -g "$owner_gid" -m 0700 "$generation_receipt_root"
    sync -f "$state_root"
  fi
  [ "$(stat -c '%u:%g:%a' "$generation_receipt_root")" \
    = "$owner_uid:$owner_gid:700" ] || {
    printf '%s\n' 'Legacy quarantine receipt directory metadata is invalid' >&2
    exit 1
  }
}

validate_generation_receipt() {
  local path=$1
  local filename=${path##*/}
  local receipt_generation=${filename%.receipt}
  [[ "$receipt_generation" =~ ^manual-quarantine-[0-9]{8}T[0-9]{6}Z-[0-9]+$ ]] \
    || { printf '%s\n' 'Legacy quarantine receipt is invalid' >&2; exit 1; }
  [ -f "$path" ] && [ ! -L "$path" ] \
    && [ "$(stat -c '%u:%g:%a:%h' "$path")" = "$owner_uid:$owner_gid:600:1" ] \
    && [ "$(wc -l <"$path")" -eq 3 ] \
    && grep -Fxq 'schema_version=1' "$path" \
    && grep -Fxq "generation=$receipt_generation" "$path" \
    || { printf '%s\n' 'Legacy quarantine receipt is invalid' >&2; exit 1; }
  generation_status=$(sed -n 's/^status=//p' "$path")
  case "$generation_status" in pending|complete|retired) ;; *)
    printf '%s\n' 'Legacy quarantine receipt is invalid' >&2
    exit 1
  esac
  generation=$receipt_generation
}

write_generation_receipt() {
  local status_value=$1
  local temporary
  ensure_generation_receipt_root
  generation_receipt=$generation_receipt_root/$generation.receipt
  temporary=$(mktemp "$generation_receipt_root/.receipt.XXXXXX")
  printf '%s\n' \
    'schema_version=1' \
    "generation=$generation" \
    "status=$status_value" >"$temporary"
  chown "$owner_uid:$owner_gid" "$temporary"
  chmod 0600 "$temporary"
  sync -f "$temporary"
  mv -T -- "$temporary" "$generation_receipt"
  sync -f "$generation_receipt_root"
  generation_status=$status_value
}

report_generation_receipts() {
  local path
  local observed_generation
  pending_generation=
  ensure_generation_receipt_root
  for path in "$generation_receipt_root"/*.receipt; do
    [ -e "$path" ] || [ -L "$path" ] || continue
    validate_generation_receipt "$path"
    observed_generation=$generation
    case "$generation_status" in
      pending)
        [ -z "$pending_generation" ] \
          || { printf '%s\n' 'Multiple legacy quarantine recoveries are pending' >&2; exit 1; }
        pending_generation=$observed_generation
        printf 'Legacy quarantine generation requires recovery: %s\n' \
          "$observed_generation" >&2
        ;;
      complete)
        [ -d "$quarantine_root/$observed_generation" ] \
          && [ ! -L "$quarantine_root/$observed_generation" ] || {
          printf '%s\n' 'Legacy quarantine generation receipt is ambiguous' >&2
          exit 1
        }
        printf 'Legacy quarantine generation pending retirement: %s\n' \
          "$observed_generation"
        ;;
      retired)
        [ ! -e "$quarantine_root/$observed_generation" ] || {
          printf '%s\n' 'Retired legacy quarantine generation still exists' >&2
          exit 1
        }
        ;;
    esac
  done
  generation=
  generation_status=
}

scan_credential_paths() {
  local rc=0
  : >"$matches"
  : >"$errors"
  grep -rlZE "$credential_pattern" "$source_root" >"$matches" 2>"$errors" || rc=$?
  if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
    printf '%s\n' 'Hermes credential scan failed closed' >&2
    exit 1
  fi
  if [ -s "$errors" ]; then
    printf '%s\n' 'Hermes credential scan reported an I/O error' >&2
    exit 1
  fi
}

classify_credential_paths() {
  live_matches=0
  legacy_matches=0
  unexpected_matches=0
  while IFS= read -r -d '' match; do
    case "$match" in
      "$source_root/.env") live_matches=$((live_matches + 1)) ;;
      "$source_root"/.nblb-cutover-backup-*/*) legacy_matches=$((legacy_matches + 1)) ;;
      "$source_root"/sessions/request_dump_*.json) legacy_matches=$((legacy_matches + 1)) ;;
      *) unexpected_matches=$((unexpected_matches + 1)) ;;
    esac
  done <"$matches"
}

validate_direct_upstream_env() {
  python3 - "$source_root/.env" <<'PY'
import re
import stat
import sys
from pathlib import Path

path = Path(sys.argv[1])
metadata = path.lstat()
if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1:
    raise SystemExit('Hermes live credential file is unsafe')
payload = path.read_bytes()
matches = re.findall(rb'nvapi-[A-Za-z0-9_-]{20,}', payload)
assignments = [line for line in payload.splitlines() if line.startswith(b'NVIDIA_API_KEY=')]
if (
    len(matches) != 1
    or len(assignments) != 1
    or assignments[0] != b'NVIDIA_API_KEY=' + matches[0]
):
    raise SystemExit('Hermes live credential classification is invalid')
PY
}

report_live_credential_state() {
  scan_credential_paths
  classify_credential_paths
  [ "$legacy_matches" -eq 0 ] && [ "$unexpected_matches" -eq 0 ] \
    || { printf '%s\n' 'Hermes legacy credential quarantine is incomplete' >&2; exit 1; }
  if [ "$live_matches" -eq 0 ]; then
    printf '%s\n' 'Hermes credential matches after quarantine: 0'
  elif [ "$live_matches" -eq 1 ]; then
    validate_direct_upstream_env
    printf '%s\n' 'Hermes legacy credential matches after quarantine: 0'
    printf '%s\n' 'Hermes live credential generation: direct-upstream'
  else
    printf '%s\n' 'Hermes live credential classification is invalid' >&2
    exit 1
  fi
}

mark_generation_receipt_retired() {
  local retired_generation=$generation
  local path
  ensure_generation_receipt_root
  path=$generation_receipt_root/$retired_generation.receipt
  if [ ! -e "$path" ]; then
    return
  fi
  validate_generation_receipt "$path"
  [ "$generation" = "$retired_generation" ] \
    || { printf '%s\n' 'Legacy quarantine receipt generation mismatch' >&2; exit 1; }
  case "$generation_status" in
    complete) write_generation_receipt retired ;;
    retired) ;;
    *) printf '%s\n' 'Pending legacy quarantine cannot be retired' >&2; exit 1 ;;
  esac
}

if [ "$command" = retire ]; then
  [[ "$generation" =~ ^manual-quarantine-[0-9]{8}T[0-9]{6}Z-[0-9]+$ ]] || {
    printf '%s\n' 'Legacy quarantine generation ID is invalid' >&2
    exit 64
  }
  target=$quarantine_root/$generation
  receipt_dir=$state_root/legacy-retirements
  if [ -L "$receipt_dir" ] || { [ -e "$receipt_dir" ] && [ ! -d "$receipt_dir" ]; }; then
    printf '%s\n' 'Legacy retirement receipt directory is unsafe' >&2
    exit 1
  fi
  if [ ! -e "$receipt_dir" ]; then
    install -d -o "$owner_uid" -g "$owner_gid" -m 0700 "$receipt_dir"
    sync -f "$state_root"
  fi
  [ "$(stat -c '%u:%g:%a' "$receipt_dir")" = "$owner_uid:$owner_gid:700" ] || {
    printf '%s\n' 'Legacy retirement receipt directory metadata is invalid' >&2
    exit 1
  }
  receipt=$receipt_dir/$generation.receipt
  validate_retirement_receipt() {
    [ -f "$receipt" ] && [ ! -L "$receipt" ] \
      && [ "$(stat -c '%u:%g:%a:%h' "$receipt")" = "$owner_uid:$owner_gid:600:1" ] \
      || { printf '%s\n' 'Legacy retirement receipt is invalid' >&2; exit 1; }
    [ "$(wc -l <"$receipt")" -eq 4 ] \
      && grep -Fxq 'schema_version=1' "$receipt" \
      && grep -Fxq "generation=$generation" "$receipt" \
      && grep -Fxq 'provider_revocation_confirmed=true' "$receipt" \
      || { printf '%s\n' 'Legacy retirement receipt is invalid' >&2; exit 1; }
    receipt_status=$(sed -n 's/^status=//p' "$receipt")
    case "$receipt_status" in pending|complete) ;; *)
      printf '%s\n' 'Legacy retirement receipt is invalid' >&2
      exit 1
    esac
  }
  write_retirement_receipt() {
    local status_value=$1
    local temporary
    temporary=$(mktemp "$receipt_dir/.receipt.XXXXXX")
    printf '%s\n' \
      'schema_version=1' \
      "generation=$generation" \
      'provider_revocation_confirmed=true' \
      "status=$status_value" >"$temporary"
    chown "$owner_uid:$owner_gid" "$temporary"
    chmod 0600 "$temporary"
    sync -f "$temporary"
    mv -T -- "$temporary" "$receipt"
    sync -f "$receipt_dir"
  }
  receipt_status=
  if [ -e "$receipt" ]; then
    validate_retirement_receipt
  fi
  if [ "$receipt_status" = complete ]; then
    [ ! -e "$target" ] \
      || { printf '%s\n' 'Legacy retirement state is ambiguous' >&2; exit 1; }
    mark_generation_receipt_retired
    printf 'Retired legacy quarantine generation: %s\n' "$generation"
    exit 0
  fi
  if [ ! -e "$target" ]; then
    if [ "$receipt_status" = pending ]; then
      sync -f "$quarantine_root"
      [ ! -e "$target" ] \
        || { printf '%s\n' 'Legacy retirement state is ambiguous' >&2; exit 1; }
      write_retirement_receipt complete
      mark_generation_receipt_retired
      printf 'Retired legacy quarantine generation: %s\n' "$generation"
      exit 0
    fi
    printf '%s\n' 'Legacy quarantine generation does not exist' >&2
    exit 1
  fi
  [ -d "$target" ] && [ ! -L "$target" ] \
    || { printf '%s\n' 'Legacy quarantine generation is unsafe' >&2; exit 1; }
  validate_tree "$target"
  reject_nested_mounts "$target"
  validate_private_tree "$target"
  if [ -z "$receipt_status" ]; then
    write_retirement_receipt pending
  fi
  find -P "$target" -depth -delete
  [ ! -e "$target" ] || {
    printf '%s\n' 'Legacy quarantine retirement is incomplete' >&2
    exit 1
  }
  sync -f "$quarantine_root"
  write_retirement_receipt complete
  mark_generation_receipt_retired
  printf 'Retired legacy quarantine generation: %s\n' "$generation"
  exit 0
fi

report_generation_receipts

for path in "$source_root"/.nblb-cutover-backup-*; do
  [ -e "$path" ] || continue
  [ -d "$path" ] || {
    printf '%s\n' 'Legacy cutover backup path is not a directory' >&2
    exit 1
  }
  validate_tree "$path"
  reject_nested_mounts "$path"
done

for path in "$source_root"/sessions/request_dump_*.json; do
  [ -e "$path" ] || continue
  [ -f "$path" ] && [ ! -L "$path" ] && [ "$(stat -c '%h' -- "$path")" -eq 1 ] || {
    printf '%s\n' 'Legacy request dump path is unsafe' >&2
    exit 1
  }
  reject_nested_mounts "$path"
done

scan_credential_paths
classify_credential_paths
[ "$unexpected_matches" -eq 0 ] \
  || { printf '%s\n' 'Unexpected Hermes credential artifact detected' >&2; exit 1; }

artifact_count=0
for path in "$source_root"/.nblb-cutover-backup-* \
  "$source_root"/sessions/request_dump_*.json; do
  [ -e "$path" ] || continue
  artifact_count=$((artifact_count + 1))
done

if [ -n "$pending_generation" ]; then
  generation=$pending_generation
  generation_receipt=$generation_receipt_root/$generation.receipt
  validate_generation_receipt "$generation_receipt"
  [ "$generation_status" = pending ] \
    || { printf '%s\n' 'Legacy quarantine recovery receipt is invalid' >&2; exit 1; }
elif [ "$artifact_count" -gt 0 ]; then
  stamp=$(date -u +%Y%m%dT%H%M%SZ)
  generation=manual-quarantine-$stamp-$$
  write_generation_receipt pending
else
  report_live_credential_state
  exit 0
fi

destination=$quarantine_root/$generation
if [ -L "$destination" ] || { [ -e "$destination" ] && [ ! -d "$destination" ]; }; then
  printf '%s\n' 'Legacy quarantine recovery destination is unsafe' >&2
  exit 1
fi
if [ ! -e "$destination" ]; then
  install -d -o "$owner_uid" -g "$owner_gid" -m 0700 \
    "$destination" "$destination/sessions"
else
  if [ ! -e "$destination/sessions" ]; then
    install -d -o "$owner_uid" -g "$owner_gid" -m 0700 "$destination/sessions"
  fi
  [ -d "$destination/sessions" ] && [ ! -L "$destination/sessions" ] \
    || { printf '%s\n' 'Legacy quarantine recovery destination is unsafe' >&2; exit 1; }
  validate_tree "$destination"
  reject_nested_mounts "$destination"
fi
chown -R --no-dereference "$owner_uid:$owner_gid" "$destination"
find -P "$destination" -type d -exec chmod 0700 {} +
find -P "$destination" -type f -exec chmod 0600 {} +
validate_private_tree "$destination"
sync -f "$destination"
sync -f "$quarantine_root"

for path in "$source_root"/.nblb-cutover-backup-*; do
  [ -e "$path" ] || continue
  validate_tree "$path"
  reject_nested_mounts "$path"
  [ ! -e "$destination/${path##*/}" ] \
    || { printf '%s\n' 'Legacy quarantine recovery has a path collision' >&2; exit 1; }
  mv -- "$path" "$destination/"
done

for path in "$source_root"/sessions/request_dump_*.json; do
  [ -e "$path" ] || continue
  [ -f "$path" ] && [ ! -L "$path" ] && [ "$(stat -c '%h' -- "$path")" -eq 1 ] \
    || { printf '%s\n' 'Legacy request dump path is unsafe' >&2; exit 1; }
  reject_nested_mounts "$path"
  [ ! -e "$destination/sessions/${path##*/}" ] \
    || { printf '%s\n' 'Legacy quarantine recovery has a path collision' >&2; exit 1; }
  mv -- "$path" "$destination/sessions/"
done

chown -R --no-dereference "$owner_uid:$owner_gid" "$destination"
find -P "$destination" -type d -exec chmod 0700 {} +
find -P "$destination" -type f -exec chmod 0600 {} +
validate_tree "$destination"
validate_private_tree "$destination"
sync -f "$destination"
sync -f "$quarantine_root"

report_live_credential_state
write_generation_receipt complete
printf 'Legacy quarantine generation: %s\n' "$generation"
