#!/usr/bin/env bash
set -Eeuo pipefail
set +x

if [ "$(id -u)" -ne 0 ]; then
  printf '%s\n' 'agent-apps delayed update lock wrapper requires root' >&2
  exit 77
fi

state_root=/opt/nvidia-build-lb/hermes-cutover-state
lock_path=$state_root/cutover.lock
if [ -L "$state_root" ]; then
  printf '%s\n' 'cutover state root is unsafe' >&2
  exit 1
fi
if [ ! -e "$state_root" ]; then
  install -d -o root -g root -m 0700 "$state_root"
fi
[ -d "$state_root" ] && [ ! -L "$state_root" ] \
  && [ "$(stat -c '%u:%g:%a' "$state_root")" = 0:0:700 ] \
  || { printf '%s\n' 'cutover state root is unsafe' >&2; exit 1; }
if [ -L "$lock_path" ]; then
  printf '%s\n' 'cutover lock is unsafe' >&2
  exit 1
fi
if [ ! -e "$lock_path" ]; then
  install -o root -g root -m 0600 /dev/null "$lock_path"
fi
[ -f "$lock_path" ] && [ ! -L "$lock_path" ] \
  && [ "$(stat -c '%u:%g:%a:%h' "$lock_path")" = 0:0:600:1 ] \
  || { printf '%s\n' 'cutover lock metadata is unsafe' >&2; exit 1; }

exec 9<>"$lock_path"
flock -x 9
exec /opt/agent-apps/bin/check-delayed-updates --apply
